use std::path::PathBuf;
use std::sync::Arc;

use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::test_util::{
    MockBanService, MockConfigService, MockLoadBalancerService, MockPlayerRegistry,
};
use tokio_util::sync::CancellationToken;

use crate::event_bus::EventBusImpl;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::plugin::PluginRegistryImpl;
use crate::plugin::manager::PluginServices;
use crate::provider::plugin_adapter::PluginProviderActivator;
use crate::routing::DomainRouter;
use crate::services::command_manager::CommandManagerImpl;
use crate::services::scheduler::SchedulerImpl;
use crate::services::server_manager_bridge::NoopServerManager;

impl PluginServices {
    #[must_use]
    pub fn for_tests() -> Self {
        Self::for_tests_with(Arc::new(EventBusImpl::new()))
    }

    #[must_use]
    pub fn for_tests_with(event_bus: Arc<EventBusImpl>) -> Self {
        let (provider_events, _) = tokio::sync::mpsc::channel(1);
        Self {
            event_bus,
            player_registry: Arc::new(MockPlayerRegistry::new()),
            server_manager: Arc::new(NoopServerManager),
            ban_service: Arc::new(MockBanService::new()),
            command_manager: Arc::new(CommandManagerImpl::new()),
            scheduler: Arc::new(SchedulerImpl::new()),
            config_service: Arc::new(MockConfigService::new()),
            load_balancer_service: Arc::new(MockLoadBalancerService::new()),
            plugin_registry: Arc::new(PluginRegistryImpl::new()),
            codec_filter_registry: Arc::new(CodecFilterRegistryImpl::new()),
            transport_filter_registry: Arc::new(TransportFilterRegistryImpl::new()),
            provider_activator: Arc::new(PluginProviderActivator::new(
                provider_events,
                Arc::new(DomainRouter::new()),
                CancellationToken::new(),
            )),
            proxy_shutdown: CancellationToken::new(),
            proxy_info: ProxyInfo::default(),
            plugins_dir: PathBuf::from("plugins"),
        }
    }
}
