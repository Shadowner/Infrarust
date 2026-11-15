use std::collections::HashMap;

use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::event::BoxFuture;
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::permissions::CapabilitySet;
use infrarust_core::plugin::PluginContextFactoryImpl;

use crate::bindings::infrarust::plugin::{command_manager, types as wt};
use crate::config::SandboxLimits;
use crate::store_state::{PluginStoreState, build_probe_state};

use super::{command_spec, is_limit_exceeded, quota_state, quotas, services};

#[tokio::test]
async fn the_command_quota_lets_a_plugin_replace_a_command_and_unregistering_frees_room() {
    let mut state = quota_state(quotas(|q| q.commands = 2), &[]);
    let named = |name: &str| command_manager::CommandSpec {
        name: name.to_owned(),
        ..command_spec()
    };
    for name in ["alpha", "beta"] {
        command_manager::Host::register(&mut state, named(name), 1)
            .await
            .unwrap()
            .unwrap();
    }
    let over = command_manager::Host::register(&mut state, named("gamma"), 2)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "commands"), "{over:?}");
    assert!(
        command_manager::Host::register(&mut state, named("Beta"), 3)
            .await
            .unwrap()
            .is_ok(),
        "registering a held name again replaces it"
    );

    assert_eq!(
        command_manager::Host::unregister(&mut state, "alpha".into())
            .await
            .unwrap(),
        Ok(())
    );
    assert!(
        command_manager::Host::register(&mut state, named("gamma"), 4)
            .await
            .unwrap()
            .is_ok()
    );
}

struct Builtin;

impl CommandHandler for Builtin {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

fn command_state(factory: &PluginContextFactoryImpl, plugin_id: &str) -> PluginStoreState {
    build_probe_state(plugin_id.to_owned(), &SandboxLimits::default())
        .with_capabilities(CapabilitySet::baseline())
        .with_ctx(factory.create_context(plugin_id))
}

fn spec_with_aliases(name: &str, aliases: &[&str]) -> command_manager::CommandSpec {
    command_manager::CommandSpec {
        name: name.to_owned(),
        aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
        ..command_spec()
    }
}

fn label_of(info: &command_manager::CommandInfo) -> String {
    info.plugin_id.as_ref().map_or_else(
        || info.spec.name.clone(),
        |plugin| format!("{plugin}:{}", info.spec.name),
    )
}

fn labels_of(infos: &[command_manager::CommandInfo]) -> Vec<String> {
    infos.iter().map(label_of).collect()
}

#[tokio::test]
async fn command_lookups_see_every_command_and_list_owned_answers_only_the_callers() {
    let shared = services(vec![]);
    shared
        .command_manager
        .register_builtin(CommandSpec::new("infrarust").alias("ir"), Box::new(Builtin));
    let factory = PluginContextFactoryImpl::for_tests(shared, HashMap::new());
    let mut guest = command_state(&factory, "guest");
    let mut other = command_state(&factory, "other");
    let warp = command_manager::CommandSpec {
        description: "Teleport to a warp".to_owned(),
        usage: Some("/warp <name>".to_owned()),
        permission: Some("warps.use".to_owned()),
        hidden: true,
        ..spec_with_aliases("Warp", &["W", "ir"])
    };
    command_manager::Host::register(&mut other, warp, 1)
        .await
        .unwrap()
        .unwrap();
    command_manager::Host::register(&mut guest, spec_with_aliases("home", &["h"]), 2)
        .await
        .unwrap()
        .unwrap();

    macro_rules! found {
        ($call:ident, $state:expr, $label:literal) => {
            command_manager::Host::$call(&mut $state, $label.into())
                .await
                .unwrap()
                .unwrap()
                .as_ref()
                .map(label_of)
        };
    }
    assert_eq!(
        command_manager::Host::get(&mut guest, "w".into())
            .await
            .unwrap(),
        Ok(Some(command_manager::CommandInfo {
            spec: command_manager::CommandSpec {
                name: "warp".to_owned(),
                aliases: vec!["w".to_owned()],
                description: "Teleport to a warp".to_owned(),
                usage: Some("/warp <name>".to_owned()),
                permission: Some("warps.use".to_owned()),
                hidden: true,
            },
            plugin_id: Some("other".to_owned()),
        })),
        "another plugin's command arrives with its whole spec and the aliases it was granted"
    );
    assert_eq!(found!(get, guest, "WARP").as_deref(), Some("other:warp"));
    assert_eq!(
        found!(get, guest, "Other:Warp").as_deref(),
        Some("other:warp")
    );
    assert_eq!(found!(get, guest, "ir").as_deref(), Some("infrarust"));
    assert_eq!(found!(get, guest, "nope"), None);

    assert_eq!(
        found!(get_by_name, guest, "warp").as_deref(),
        Some("other:warp")
    );
    assert_eq!(
        found!(get_by_name, guest, "guest:home").as_deref(),
        Some("guest:home")
    );
    assert_eq!(
        found!(get_by_name, guest, "infrarust").as_deref(),
        Some("infrarust")
    );
    assert_eq!(found!(get_by_name, guest, "w"), None);
    assert_eq!(found!(get_by_name, guest, "ir"), None);

    assert_eq!(
        found!(get_by_alias, guest, "W").as_deref(),
        Some("other:warp")
    );
    assert_eq!(
        found!(get_by_alias, guest, "ir").as_deref(),
        Some("infrarust")
    );
    assert_eq!(
        found!(get_by_alias, other, "h").as_deref(),
        Some("guest:home")
    );
    assert_eq!(found!(get_by_alias, guest, "warp"), None);
    assert_eq!(found!(get_by_alias, guest, "other:warp"), None);

    for (label, answer) in [
        ("h", true),
        ("other:warp", true),
        ("IR", true),
        ("nope", false),
    ] {
        assert_eq!(
            command_manager::Host::contains(&mut guest, label.into())
                .await
                .unwrap(),
            Ok(answer),
            "{label}"
        );
    }

    let listed = command_manager::Host::list(&mut guest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        labels_of(&listed),
        ["guest:home", "infrarust", "other:warp"]
    );
    let owned = |infos: Result<Vec<command_manager::CommandInfo>, wt::HostError>| {
        labels_of(&infos.unwrap())
    };
    assert_eq!(
        owned(command_manager::Host::list_owned(&mut guest).await.unwrap()),
        ["guest:home"]
    );
    assert_eq!(
        owned(command_manager::Host::list_owned(&mut other).await.unwrap()),
        ["other:warp"]
    );

    command_manager::Host::unregister(&mut guest, "home".into())
        .await
        .unwrap()
        .unwrap();
    assert!(owned(command_manager::Host::list_owned(&mut guest).await.unwrap()).is_empty());
    assert_eq!(found!(get, other, "h"), None);
}
