//! Native↔WASM parity: the native stats adapter, driven through a real
//! `PluginContext`, replies to `/count` with the exact string the wasm adapter
//! produces (asserted in `wasm_fixtures::test_stats_count_command`). Both wrap
//! the same `infrarust-plugin-stats` core.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use infrarust_api::command::CommandManager;
use infrarust_api::event::bus::EventBus;
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::plugin::Plugin;
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::types::PlayerId;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::filter::transport_registry::TransportFilterRegistryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginRegistryImpl};
use infrarust_core::routing::DomainRouter;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use infrarust_plugin_stats_native::StatsPlugin;
use tokio_util::sync::CancellationToken;

mod mock_services;
use mock_services::{
    MockBanService, MockConfigService, MockLoadBalancerService, MockPlayerRegistry,
    RecordingPlayerRegistry,
};

#[tokio::test(flavor = "multi_thread")]
async fn native_count_command_matches_wasm() {
    let command_manager = Arc::new(CommandManagerImpl::new());
    let services = PluginServices {
        event_bus: Arc::new(EventBusImpl::new()) as Arc<dyn EventBus>,
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::clone(&command_manager) as Arc<dyn CommandManager>,
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        load_balancer_service: Arc::new(MockLoadBalancerService),
        plugin_registry: Arc::new(PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(CodecFilterRegistryImpl::new()),
        transport_filter_registry: Arc::new(TransportFilterRegistryImpl::new()),
        domain_router: Arc::new(DomainRouter::new()),
        proxy_shutdown: CancellationToken::new(),
        proxy_info: ProxyInfo::default(),
        plugins_dir: std::env::temp_dir(),
    };
    let factory = PluginContextFactoryImpl::new(services, HashMap::new());

    let ctx = factory.create_context("stats");
    StatsPlugin
        .on_enable(ctx.as_ref())
        .await
        .expect("native stats enables");

    let sent = Arc::new(Mutex::new(Vec::new()));
    let registry = RecordingPlayerRegistry {
        count: 7,
        sent: Arc::clone(&sent),
    };
    let found = command_manager
        .dispatch(Some(PlayerId::new(1)), "count", &registry)
        .await;
    assert!(found, "native 'count' command should be registered");

    assert_eq!(
        sent.lock().unwrap().as_slice(),
        ["Online: 7".to_string()],
        "native /count replies identically to the wasm build"
    );
}
