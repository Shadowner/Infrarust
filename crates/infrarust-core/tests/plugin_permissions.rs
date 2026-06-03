#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::{HandlerResult, LimboHandler, LimboSession};
use infrarust_api::permissions::Capability;
use infrarust_api::plugin::PluginContext;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::PluginPermissions;
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::plugin::context_factory::{PluginContextFactory, PluginContextFactoryImpl};
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;

mod mock_services;
use mock_services::{MockBanService, MockConfigService, MockPlayerRegistry};

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

fn build_services() -> PluginServices {
    PluginServices {
        event_bus: Arc::new(EventBusImpl::new()) as Arc<dyn infrarust_api::event::bus::EventBus>,
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        plugin_registry: Arc::new(infrarust_core::plugin::PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        domain_router: Arc::new(infrarust_core::routing::DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        proxy_info: infrarust_api::services::proxy_info::ProxyInfo::default(),
        plugins_dir: PathBuf::from("plugins"),
    }
}

fn factory(entries: Vec<(&str, PluginPermissions)>) -> PluginContextFactoryImpl {
    let map: HashMap<String, PluginPermissions> = entries
        .into_iter()
        .map(|(id, p)| (id.to_string(), p))
        .collect();
    PluginContextFactoryImpl::new(build_services(), map)
}

fn perms(strings: &[&str], trusted: bool) -> PluginPermissions {
    PluginPermissions {
        permissions: strings.iter().map(|s| (*s).to_string()).collect(),
        trusted,
    }
}

fn limbo_handlers_after_register(ctx: &Arc<dyn PluginContext>) -> usize {
    ctx.register_limbo_handler(Box::new(DummyLimbo));
    ctx.as_any()
        .downcast_ref::<PluginContextImpl>()
        .expect("real PluginContextImpl")
        .take_limbo_handlers()
        .len()
}

#[test]
fn untrusted_plugin_without_caps_is_gated_to_baseline() {
    let f = factory(vec![("p", perms(&[], false))]);
    let ctx = f.create_context("p");
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
    let ctx = f.create_context("not-in-map");
    assert!(ctx.capabilities().has(Capability::EventBus));
    assert!(!ctx.capabilities().has(Capability::CodecFilter));
    assert!(ctx.codec_filters().is_none());
}

#[test]
fn config_capability_reflected_in_capabilities() {
    let f = factory(vec![("p", perms(&["server-manage"], false))]);
    let ctx = f.create_context("p");
    assert!(ctx.capabilities().has(Capability::ServerManage));
    assert!(
        ctx.capabilities().has(Capability::EventBus),
        "baseline must be preserved alongside opt-ins"
    );
}

#[test]
fn codec_filter_capability_unlocks_registry() {
    let denied = factory(vec![("p", perms(&[], false))]);
    assert!(denied.create_context("p").codec_filters().is_none());

    let granted = factory(vec![("p", perms(&["codec-filter"], false))]);
    assert!(granted.create_context("p").codec_filters().is_some());
}

#[test]
fn transport_filter_never_granted_via_config() {
    let f = factory(vec![("p", perms(&["transport-filter"], false))]);
    let ctx = f.create_context("p");
    assert!(!ctx.capabilities().has(Capability::TransportFilter));
    assert!(ctx.transport_filters().is_none());
}

#[test]
fn limbo_capability_allows_registration() {
    let f = factory(vec![("p", perms(&["limbo"], false))]);
    let ctx = f.create_context("p");
    assert_eq!(limbo_handlers_after_register(&ctx), 1);
}

#[test]
fn trusted_native_plugin_gets_full_set() {
    let f = factory(vec![("native", perms(&[], true))]);
    let ctx = f.create_context("native");
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
    let ctx = f.create_context("auth");
    assert!(ctx.capabilities().has(Capability::Limbo));
    assert_eq!(limbo_handlers_after_register(&ctx), 1);
}
