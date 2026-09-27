#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::{HandlerResult, LimboHandler, LimboSession};
use infrarust_api::permissions::Capability;
use infrarust_api::plugin::PluginContext;
use infrarust_core::plugin::PluginPermissions;
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::plugin::context_factory::PluginContextFactoryImpl;
use infrarust_core::plugin::manager::PluginServices;

struct DummyLimbo;

impl LimboHandler for DummyLimbo {
    fn name(&self) -> &str {
        "dummy"
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async { HandlerResult::Hold })
    }
}

fn factory_in(
    plugins_dir: &Path,
    entries: Vec<(&str, PluginPermissions)>,
) -> PluginContextFactoryImpl {
    let map: HashMap<String, PluginPermissions> = entries
        .into_iter()
        .map(|(id, p)| (id.to_string(), p))
        .collect();
    let services = PluginServices {
        plugins_dir: plugins_dir.to_path_buf(),
        ..PluginServices::for_tests()
    };
    PluginContextFactoryImpl::new(services, map)
}

fn factory(entries: Vec<(&str, PluginPermissions)>) -> PluginContextFactoryImpl {
    factory_in(Path::new("plugins"), entries)
}

fn perms(strings: &[&str], trusted: bool) -> PluginPermissions {
    PluginPermissions {
        permissions: strings.iter().map(|s| (*s).to_string()).collect(),
        deny: Vec::new(),
        trusted,
    }
}

fn denying(mut perms: PluginPermissions, denied: &[&str]) -> PluginPermissions {
    perms.deny = denied.iter().map(|s| (*s).to_string()).collect();
    perms
}

fn limbo_handlers_after_register(ctx: &Arc<PluginContextImpl>) -> usize {
    let registered = ctx.register_limbo_handler(Box::new(DummyLimbo));
    let count = ctx.limbo_handlers().len();
    assert_eq!(registered.is_ok(), count == 1, "{registered:?}");
    if count == 0 {
        assert_eq!(
            registered.unwrap_err(),
            infrarust_api::limbo::LimboHandlerError::MissingCapability
        );
    }
    count
}

#[test]
fn untrusted_plugin_without_caps_is_gated_to_baseline() {
    let f = factory(vec![("p", perms(&[], false))]);
    let ctx = f.context("p");
    assert!(ctx.capabilities().has(Capability::EventBus));
    assert!(ctx.capabilities().has(Capability::PlayerWrite));
    assert!(!ctx.capabilities().has(Capability::ServerManage));
    assert!(ctx.codec_filters().is_none());
    assert!(ctx.transport_filters().is_none());
    assert_eq!(limbo_handlers_after_register(&ctx), 0);
}

#[test]
fn unknown_plugin_id_defaults_to_baseline() {
    let f = factory(vec![]);
    let ctx = f.context("not-in-map");
    assert!(ctx.capabilities().has(Capability::EventBus));
    assert!(!ctx.capabilities().has(Capability::CodecFilter));
    assert!(ctx.codec_filters().is_none());
}

#[test]
fn config_capability_reflected_in_capabilities() {
    let f = factory(vec![("p", perms(&["server-manage"], false))]);
    let ctx = f.context("p");
    assert!(ctx.capabilities().has(Capability::ServerManage));
    assert!(
        ctx.capabilities().has(Capability::EventBus),
        "baseline must be preserved alongside opt-ins"
    );
}

#[test]
fn codec_filter_capability_unlocks_registry() {
    let denied = factory(vec![("p", perms(&[], false))]);
    assert!(denied.context("p").codec_filters().is_none());

    let granted = factory(vec![("p", perms(&["codec-filter"], false))]);
    assert!(granted.context("p").codec_filters().is_some());
}

#[test]
fn transport_filter_never_granted_via_config() {
    let f = factory(vec![("p", perms(&["transport-filter"], false))]);
    let ctx = f.context("p");
    assert!(!ctx.capabilities().has(Capability::TransportFilter));
    assert!(ctx.transport_filters().is_none());
}

#[test]
fn limbo_capability_allows_registration() {
    let f = factory(vec![("p", perms(&["limbo"], false))]);
    let ctx = f.context("p");
    assert_eq!(limbo_handlers_after_register(&ctx), 1);
}

#[test]
fn trusted_native_plugin_gets_full_set() {
    let f = factory(vec![("native", perms(&[], true))]);
    let ctx = f.context("native");
    assert!(ctx.capabilities().has(Capability::TransportFilter));
    assert!(ctx.capabilities().has(Capability::CodecFilter));
    assert!(ctx.capabilities().has(Capability::Limbo));
    assert!(ctx.codec_filters().is_some());
    assert!(ctx.transport_filters().is_some());
    assert_eq!(limbo_handlers_after_register(&ctx), 1);
}

#[test]
fn trusted_wins_over_conflicting_config_so_native_keeps_limbo() {
    let f = factory(vec![("auth", perms(&["ban"], true))]);
    let ctx = f.context("auth");
    assert!(ctx.capabilities().has(Capability::Limbo));
    assert_eq!(limbo_handlers_after_register(&ctx), 1);
}

#[test]
fn data_dir_is_created_on_demand() {
    let tmp = tempfile::tempdir().unwrap();
    let f = factory_in(tmp.path(), vec![("p", perms(&[], false))]);

    let dir = f.context("p").data_dir();

    assert_eq!(dir, tmp.path().join("p"));
    assert!(
        dir.is_dir(),
        "data_dir() must create the directory it returns"
    );
}

#[test]
fn data_dir_creates_missing_plugins_dir_too() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().join("absent").join("plugins");
    let f = factory_in(&plugins_dir, vec![("admin_api", perms(&[], true))]);

    assert!(f.context("admin_api").data_dir().is_dir());
}

#[test]
fn data_dir_never_joins_an_id_that_breaks_the_plugin_id_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir(&plugins_dir).unwrap();
    let outside = tmp.path().join("outside");
    let hostile = [
        "../escape".to_owned(),
        "..".to_owned(),
        outside.display().to_string(),
        ".cache".to_owned(),
        "a/b".to_owned(),
        "Upper".to_owned(),
        String::new(),
    ];
    let f = factory_in(&plugins_dir, Vec::new());
    for id in &hostile {
        let dir = f.context(id).data_dir();
        assert!(
            dir.starts_with(&plugins_dir) && dir != plugins_dir,
            "{id:?} got {}",
            dir.display()
        );
        assert!(!dir.exists(), "{id:?} created {}", dir.display());
    }
    let created: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        created,
        ["plugins"],
        "nothing is created next to plugins_dir"
    );
    assert_eq!(
        std::fs::read_dir(&plugins_dir).unwrap().count(),
        0,
        "nothing is created inside plugins_dir either"
    );
}

#[test]
fn data_dir_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let f = factory_in(tmp.path(), vec![("p", perms(&[], false))]);
    let ctx = f.context("p");

    let dir = ctx.data_dir();
    std::fs::write(dir.join("state.json"), b"{}").unwrap();

    assert_eq!(ctx.data_dir(), dir);
    assert!(
        dir.join("state.json").exists(),
        "a second call must not disturb what the plugin already wrote"
    );
}

#[test]
fn denied_capabilities_are_removed_after_baseline_and_grants() {
    let f = factory(vec![(
        "p",
        denying(
            perms(&["ban", "limbo"], false),
            &["player-write", "ban", "not-a-capability"],
        ),
    )]);
    let caps = f.context("p").capabilities().clone();
    assert!(
        !caps.has(Capability::PlayerWrite),
        "baseline capability denied"
    );
    assert!(!caps.has(Capability::Ban), "a deny wins over a grant");
    assert!(caps.has(Capability::Limbo), "other grants are kept");
    assert!(
        caps.has(Capability::EventBus),
        "other baseline capabilities are kept"
    );
}

#[test]
fn denied_capabilities_are_removed_from_trusted_plugins_too() {
    let f = factory(vec![("t", denying(perms(&[], true), &["codec-filter"]))]);
    let ctx = f.context("t");
    assert!(!ctx.capabilities().has(Capability::CodecFilter));
    assert!(ctx.codec_filters().is_none());
    assert!(ctx.capabilities().has(Capability::ServerManage));
}
