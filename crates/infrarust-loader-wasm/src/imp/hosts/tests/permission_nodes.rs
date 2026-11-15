use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use infrarust_api::loader::PluginContextFactory;
use infrarust_api::permissions::{
    CapabilitySet, PermissionDefault, PermissionNode, PermissionSubject, Tristate,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::{GameProfile, PlayerId};
use infrarust_config::{PermissionsConfig, WasmQuotasConfig};
use infrarust_core::permissions::PermissionService;
use infrarust_core::plugin::PluginContextFactoryImpl;

use crate::bindings::infrarust::plugin::{permission_nodes, types as wt};
use crate::config::SandboxLimits;
use crate::store_state::{PluginStoreState, build_probe_state};

use super::{is_limit_exceeded, quotas, services};

struct NodeHost {
    permissions: Arc<PermissionService>,
    factory: PluginContextFactoryImpl,
}

impl NodeHost {
    fn new() -> Self {
        let permissions = Arc::new(PermissionService::new_sync(&PermissionsConfig::default()));
        let factory = PluginContextFactoryImpl::for_tests(services(vec![]), HashMap::new())
            .with_permissions(Arc::clone(&permissions));
        Self {
            permissions,
            factory,
        }
    }

    fn wasm(&self, plugin_id: &str, quotas: WasmQuotasConfig) -> PluginStoreState {
        let sandbox = SandboxLimits {
            quotas,
            ..SandboxLimits::default()
        };
        build_probe_state(plugin_id.to_owned(), &sandbox)
            .with_capabilities(CapabilitySet::default())
            .with_ctx(self.factory.create_context(plugin_id))
    }

    fn native(&self, plugin_id: &str, name: &str, default: PermissionDefault) {
        self.factory
            .context(plugin_id)
            .register_permission_node(PermissionNode::new(name, default))
            .unwrap();
    }

    async fn player_value(&self, node: &str) -> Tristate {
        let steve = PermissionSubject::player(
            PlayerId::new(7),
            GameProfile {
                uuid: uuid::Uuid::from_u128(7),
                username: "Steve".to_owned(),
                properties: vec![],
            },
            true,
            SocketAddr::from(([203, 0, 113, 7], 51234)),
        );
        let checker = self.permissions.create_checker(&steve).await;
        self.permissions.value(checker.as_ref(), node)
    }
}

fn wit_node(
    name: &str,
    default: permission_nodes::PermissionDefault,
) -> permission_nodes::PermissionNode {
    permission_nodes::PermissionNode {
        name: name.to_owned(),
        description: String::new(),
        default,
    }
}

async fn register_node(
    state: &mut PluginStoreState,
    node: permission_nodes::PermissionNode,
) -> Result<(), wt::HostError> {
    permission_nodes::Host::register(state, node).await.unwrap()
}

#[tokio::test]
async fn a_wasm_plugin_registers_its_own_node_and_a_player_gets_its_default() {
    let host = NodeHost::new();
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    let fly = permission_nodes::PermissionNode {
        description: "Fly in the lobby".to_owned(),
        ..wit_node(" Test.Fly ", permission_nodes::PermissionDefault::True)
    };
    assert_eq!(register_node(&mut state, fly).await, Ok(()));
    let staff = wit_node("test.staff", permission_nodes::PermissionDefault::Admin);
    assert_eq!(register_node(&mut state, staff).await, Ok(()));

    assert_eq!(host.player_value("test.fly").await, Tristate::True);
    assert_eq!(host.player_value("test.staff").await, Tristate::False);
    let info = host
        .factory
        .context("reader")
        .permission_node("test.fly")
        .unwrap();
    assert_eq!(info.plugin_id.as_deref(), Some("test"));
    assert_eq!(info.node.name, "test.fly");
    assert_eq!(info.node.description, "Fly in the lobby");
    assert_eq!(info.node.default, PermissionDefault::True);
}

#[tokio::test]
async fn a_wasm_plugin_may_only_register_nodes_in_its_own_namespace() {
    let host = NodeHost::new();
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    for name in ["otherplugin.x", "fly", "infrarust.x", "testing.x", "test"] {
        let refused = register_node(
            &mut state,
            wit_node(name, permission_nodes::PermissionDefault::True),
        )
        .await
        .unwrap_err();
        assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument, "{name}");
        assert!(
            refused.message.contains("start with 'test.'"),
            "{name}: {}",
            refused.message
        );
    }
    assert_eq!(
        host.player_value("otherplugin.x").await,
        Tristate::Undefined
    );
    assert_eq!(host.player_value("fly").await, Tristate::Undefined);
    assert_eq!(
        host.factory.context("reader").permission_nodes().len(),
        1,
        "only the proxy's own infrarust.admin is registered"
    );
    assert_eq!(
        register_node(
            &mut state,
            wit_node("TEST.fly", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(()),
        "the name is normalized before its namespace is checked"
    );
}

#[tokio::test]
async fn a_wasm_plugin_still_meets_the_native_rules_inside_its_namespace() {
    let host = NodeHost::new();
    host.native("other", "test.shared", PermissionDefault::False);
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    let taken = register_node(
        &mut state,
        wit_node("test.shared", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(taken.kind, wt::ErrorKind::Conflict);
    assert!(taken.message.contains("'other'"), "{}", taken.message);
    assert_eq!(host.player_value("test.shared").await, Tristate::False);
    let invalid = register_node(
        &mut state,
        wit_node("test.*", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.kind, wt::ErrorKind::InvalidArgument);

    let mut proxy_named = host.wasm("infrarust", WasmQuotasConfig::default());
    for name in ["infrarust.fly", "infrarust.admin"] {
        let reserved = register_node(
            &mut proxy_named,
            wit_node(name, permission_nodes::PermissionDefault::True),
        )
        .await
        .unwrap_err();
        assert_eq!(reserved.kind, wt::ErrorKind::Conflict, "{name}");
    }
    assert_eq!(
        host.player_value("infrarust.fly").await,
        Tristate::Undefined
    );
}

#[tokio::test]
async fn registering_an_owned_node_again_updates_it_and_takes_no_more_room() {
    let host = NodeHost::new();
    let one = || quotas(|q| q.permission_nodes = 1);
    let mut state = host.wasm("test", one());
    assert_eq!(
        register_node(
            &mut state,
            wit_node("test.fly", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(())
    );
    let moved = permission_nodes::PermissionNode {
        description: "Fly anywhere".to_owned(),
        ..wit_node("TEST.FLY", permission_nodes::PermissionDefault::Admin)
    };
    assert_eq!(register_node(&mut state, moved).await, Ok(()));
    let info = permission_nodes::Host::get(&mut state, "test.fly".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        info.node.default,
        permission_nodes::PermissionDefault::Admin
    );
    assert_eq!(info.node.description, "Fly anywhere");
    assert_eq!(host.player_value("test.fly").await, Tristate::False);

    let mut recovered = host.wasm("test", one());
    assert_eq!(
        register_node(
            &mut recovered,
            wit_node("test.fly", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(()),
        "a fresh instance's on_enable registers the node it still owns"
    );
    assert_eq!(host.player_value("test.fly").await, Tristate::True);
    let full = register_node(
        &mut recovered,
        wit_node("test.walk", permission_nodes::PermissionDefault::True),
    )
    .await;
    assert!(is_limit_exceeded(&full, "permission_nodes"), "{full:?}");
}

#[tokio::test]
async fn the_node_quota_counts_only_the_nodes_the_plugin_holds() {
    let host = NodeHost::new();
    host.native("other", "other.kick", PermissionDefault::Admin);
    host.native("other", "other.ban", PermissionDefault::Admin);
    let mut state = host.wasm("test", quotas(|q| q.permission_nodes = 2));
    for name in ["test.a", "test.b"] {
        assert_eq!(
            register_node(
                &mut state,
                wit_node(name, permission_nodes::PermissionDefault::True)
            )
            .await,
            Ok(()),
            "{name}"
        );
    }
    let over = register_node(
        &mut state,
        wit_node("test.c", permission_nodes::PermissionDefault::True),
    )
    .await;
    assert!(is_limit_exceeded(&over, "permission_nodes"), "{over:?}");
    assert_eq!(host.player_value("test.c").await, Tristate::Undefined);
    let foreign = register_node(
        &mut state,
        wit_node("other.c", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(foreign.kind, wt::ErrorKind::InvalidArgument);
}

#[tokio::test]
async fn get_and_list_see_the_proxys_and_every_plugins_nodes_without_any_capability() {
    let host = NodeHost::new();
    host.native("other", "other.kick", PermissionDefault::Admin);
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    register_node(
        &mut state,
        wit_node("test.fly", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap();

    let kick = permission_nodes::Host::get(&mut state, " Other.Kick ".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kick.plugin_id.as_deref(), Some("other"));
    assert_eq!(
        kick.node.default,
        permission_nodes::PermissionDefault::Admin
    );
    let admin = permission_nodes::Host::get(&mut state, "infrarust.admin".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admin.plugin_id, None);
    assert_eq!(
        admin.node.default,
        permission_nodes::PermissionDefault::False
    );
    assert_eq!(
        permission_nodes::Host::get(&mut state, "nobody.x".into())
            .await
            .unwrap(),
        None
    );
    let listed: Vec<(String, Option<String>)> = permission_nodes::Host::list(&mut state)
        .await
        .unwrap()
        .into_iter()
        .map(|info| (info.node.name, info.plugin_id))
        .collect();
    assert_eq!(
        listed,
        [
            ("infrarust.admin".to_owned(), None),
            ("other.kick".to_owned(), Some("other".to_owned())),
            ("test.fly".to_owned(), Some("test".to_owned())),
        ]
    );

    let mut inspected = build_probe_state("test".to_owned(), &SandboxLimits::default());
    let unavailable = register_node(
        &mut inspected,
        wit_node("test.walk", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(unavailable.kind, wt::ErrorKind::Unavailable);
    assert!(
        permission_nodes::Host::list(&mut inspected)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_disabled_wasm_plugin_takes_its_nodes_along() {
    let host = NodeHost::new();
    host.native("other", "other.kick", PermissionDefault::Admin);
    let mut state = host.wasm("test", quotas(|q| q.permission_nodes = 1));
    register_node(
        &mut state,
        wit_node("test.fly", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap();
    let mut reader = host.wasm("reader", WasmQuotasConfig::default());

    host.factory.context("test").cleanup();

    assert_eq!(
        permission_nodes::Host::get(&mut reader, "test.fly".into())
            .await
            .unwrap(),
        None
    );
    assert_eq!(host.player_value("test.fly").await, Tristate::Undefined);
    let names: Vec<String> = permission_nodes::Host::list(&mut reader)
        .await
        .unwrap()
        .into_iter()
        .map(|info| info.node.name)
        .collect();
    assert_eq!(names, ["infrarust.admin", "other.kick"]);
    assert_eq!(
        register_node(
            &mut state,
            wit_node("test.walk", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(()),
        "the removed node no longer counts against the quota"
    );
}
