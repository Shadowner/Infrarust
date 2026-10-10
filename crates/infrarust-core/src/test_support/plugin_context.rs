use std::path::PathBuf;
use std::sync::Arc;

use infrarust_api::command::CommandManager;
use infrarust_api::event::bus::EventBus;
use infrarust_api::filter::registry::{CodecFilterRegistry, TransportFilterRegistry};
use infrarust_api::limbo::{LimboHandler, LimboHandlerError, LimboHandlerRegistration};
use infrarust_api::messaging::{ChannelRegistrar, ServerMessenger};
use infrarust_api::permissions::{
    CapabilitySet, PermissionNode, PermissionNodeError, PermissionNodeInfo, PermissionProvider,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::provider::PluginConfigProvider;
use infrarust_api::services::ban_service::{BanProvider, BanService};
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::load_balancer::LoadBalancerService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::plugin_registry::PluginRegistry;
use infrarust_api::services::providers::ProviderRejected;
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::services::scheduler::Scheduler;
use infrarust_api::services::server_manager::ServerManager;
use infrarust_api::services::service_registry::ServiceRegistry;
use tokio_util::sync::CancellationToken;

use crate::plugin::context_factory::PluginContextFactory;

pub struct MockPluginContext {
    plugin_id: String,
    capabilities: CapabilitySet,
}

impl MockPluginContext {
    #[must_use]
    pub fn new(plugin_id: &str) -> Self {
        Self {
            plugin_id: plugin_id.to_owned(),
            capabilities: CapabilitySet::native_trusted(),
        }
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: CapabilitySet) -> Self {
        self.capabilities = capabilities;
        self
    }
}

impl infrarust_api::plugin::private::Sealed for MockPluginContext {}

impl PluginContext for MockPluginContext {
    fn event_bus(&self) -> Arc<dyn EventBus> {
        unimplemented!("mock")
    }
    fn player_registry(&self) -> Arc<dyn PlayerRegistry> {
        unimplemented!("mock")
    }
    fn server_manager(&self) -> Arc<dyn ServerManager> {
        unimplemented!("mock")
    }
    fn ban_service(&self) -> Arc<dyn BanService> {
        unimplemented!("mock")
    }
    fn register_ban_provider(
        &self,
        _provider: Arc<dyn BanProvider>,
    ) -> Result<(), ProviderRejected> {
        unimplemented!("mock")
    }
    fn register_permission_provider(
        &self,
        _provider: Arc<dyn PermissionProvider>,
    ) -> Result<(), ProviderRejected> {
        unimplemented!("mock")
    }
    fn register_permission_node(&self, _node: PermissionNode) -> Result<(), PermissionNodeError> {
        unimplemented!("mock")
    }
    fn permission_node(&self, _name: &str) -> Option<PermissionNodeInfo> {
        unimplemented!("mock")
    }
    fn permission_nodes(&self) -> Vec<PermissionNodeInfo> {
        unimplemented!("mock")
    }
    fn config_service(&self) -> Arc<dyn ConfigService> {
        unimplemented!("mock")
    }
    fn load_balancer_service(&self) -> Arc<dyn LoadBalancerService> {
        unimplemented!("mock")
    }
    fn command_manager(&self) -> Arc<dyn CommandManager> {
        unimplemented!("mock")
    }
    fn scheduler(&self) -> Arc<dyn Scheduler> {
        unimplemented!("mock")
    }
    fn services(&self) -> Arc<dyn ServiceRegistry> {
        unimplemented!("mock")
    }
    fn register_limbo_handler(
        &self,
        _handler: Box<dyn LimboHandler>,
    ) -> Result<LimboHandlerRegistration, LimboHandlerError> {
        unimplemented!("mock")
    }
    fn codec_filters(&self) -> Option<&dyn CodecFilterRegistry> {
        None
    }
    fn transport_filters(&self) -> Option<&dyn TransportFilterRegistry> {
        None
    }
    fn plugin_registry(&self) -> Arc<dyn PluginRegistry> {
        unimplemented!("mock")
    }
    fn register_config_provider(&self, _provider: Box<dyn PluginConfigProvider>) {}
    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
    fn data_dir(&self) -> PathBuf {
        PathBuf::from("plugins").join(&self.plugin_id)
    }
    fn proxy_shutdown(&self) -> CancellationToken {
        CancellationToken::new()
    }
    fn proxy_info(&self) -> &ProxyInfo {
        unimplemented!("mock")
    }
    fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }
    fn channel_registrar(&self) -> &dyn ChannelRegistrar {
        unimplemented!("mock")
    }
    fn server_messenger(&self) -> Arc<dyn ServerMessenger> {
        unimplemented!("mock")
    }
}

pub struct MockPluginContextFactory;

impl PluginContextFactory for MockPluginContextFactory {
    fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
        Arc::new(MockPluginContext::new(plugin_id))
    }
}
