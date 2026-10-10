#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::permissions::{PermissionDefault, PermissionNode, PermissionSubject, Tristate};
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::PlayerId;
use infrarust_config::PermissionsConfig;
use infrarust_core::permissions::PermissionService;
use infrarust_core::plugin::manager::PluginManager;
use infrarust_core::services::command_manager::DispatchOutcome;

use support::{EnvOptions, console, loader_from_toml, make_env_with, nil_profile, stage};

const PROBE: &str = "node-probe";

const TWO_NODES: &str = "[wasm.quotas]\npermission_nodes = 2\n";

fn log_lines(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(PROBE).join("nodes.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn enable_block(phase: &str) -> Vec<String> {
    [
        &format!("enable {phase}"),
        "register fly invalid-argument",
        "register native-owner.kick invalid-argument",
        "register infrarust.fly invalid-argument",
        "register node-probe.extra limit-exceeded",
        "get node-probe.fly Admin node-probe Fly anywhere",
        "get native-owner.kick Admin native-owner Kick players",
        "get infrarust.admin False - Every proxy command, and every node whose default is admin",
        "get nobody.node -",
        "list infrarust.admin=-,native-owner.kick=native-owner,node-probe.fly=node-probe,node-probe.use=node-probe",
    ]
    .map(str::to_owned)
    .to_vec()
}

async fn player_value(permissions: &PermissionService, node: &str) -> Tristate {
    let steve = PermissionSubject::player(
        PlayerId::new(7),
        nil_profile("Steve"),
        true,
        "203.0.113.7:51234".parse().unwrap(),
    );
    let checker = permissions.create_checker(&steve).await;
    permissions.value(checker.as_ref(), node)
}

async fn wait_for_recovery(dir: &Path) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !log_lines(dir).contains(&"enable recovered".to_owned()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the probe never recovered: {:?}", log_lines(dir)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_keeps_its_own_nodes_across_a_recovery_and_loses_them_when_disabled() {
    let (_tmp, dir) = stage(PROBE);
    let permissions = Arc::new(PermissionService::new_sync(&PermissionsConfig::default()));
    let env = make_env_with(
        dir.clone(),
        EnvOptions {
            permissions: Some(Arc::clone(&permissions)),
            ..EnvOptions::default()
        },
    );
    let commands = Arc::clone(&env.command_manager);
    let factory = Arc::new(env.factory);
    factory
        .context("native-owner")
        .register_permission_node(
            PermissionNode::new("native-owner.kick", PermissionDefault::Admin)
                .description("Kick players"),
        )
        .unwrap();
    let mut manager = PluginManager::new(vec![Box::new(loader_from_toml(TWO_NODES))]);
    manager.discover_all(&dir).await.unwrap();
    let errors = manager.load_and_enable_all(Arc::clone(&factory)).await;
    assert!(errors.is_empty(), "{errors:?}");

    assert_eq!(log_lines(&dir), enable_block("initial"));
    let reader = factory.context("reader");
    let fly = reader.permission_node("node-probe.fly").unwrap();
    assert_eq!(fly.plugin_id.as_deref(), Some(PROBE));
    assert_eq!(fly.node.default, PermissionDefault::Admin);
    assert_eq!(fly.node.description, "Fly anywhere");
    assert_eq!(
        player_value(&permissions, "node-probe.use").await,
        Tristate::True
    );
    assert_eq!(
        player_value(&permissions, "node-probe.fly").await,
        Tristate::False
    );
    assert_eq!(player_value(&permissions, "fly").await, Tristate::Undefined);

    assert_eq!(
        commands.dispatch(console(), "nodetrap").await,
        DispatchOutcome::Executed
    );
    wait_for_recovery(&dir).await;
    let mut both = enable_block("initial");
    both.extend(enable_block("recovered"));
    assert_eq!(
        log_lines(&dir),
        both,
        "the recovered on_enable registers the two nodes it owns again within a quota of two"
    );
    assert_eq!(
        reader
            .permission_node("node-probe.use")
            .unwrap()
            .plugin_id
            .as_deref(),
        Some(PROBE)
    );

    manager.disable_plugin(PROBE).await.unwrap();
    assert_eq!(reader.permission_node("node-probe.use"), None);
    assert_eq!(reader.permission_node("node-probe.fly"), None);
    assert_eq!(
        player_value(&permissions, "node-probe.use").await,
        Tristate::Undefined
    );
    assert_eq!(
        reader
            .permission_nodes()
            .into_iter()
            .map(|info| info.node.name)
            .collect::<Vec<_>>(),
        ["infrarust.admin", "native-owner.kick"]
    );
}
