use std::path::PathBuf;
use std::sync::Arc;

use infrarust_api::command::CommandManager;
use infrarust_api::event::bus::EventBus;
use infrarust_api::filter::registry::{CodecFilterRegistry, TransportFilterRegistry};
use infrarust_api::limbo::{LimboHandler, LimboHandlerError, LimboHandlerRegistration};
use infrarust_api::permissions::{
    CapabilitySet, PermissionNode, PermissionNodeError, PermissionNodeInfo, PermissionProvider,
    PermissionProviderRejected,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::provider::PluginConfigProvider;
use infrarust_api::services::ban_service::{BanProvider, BanProviderRejected, BanService};
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::load_balancer::LoadBalancerService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::plugin_registry::PluginRegistry;
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
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn event_bus(&self) -> &dyn EventBus {
        unimplemented!("mock")
    }
    fn event_bus_handle(&self) -> Arc<dyn EventBus> {
        unimplemented!("mock")
    }
    fn player_registry(&self) -> &dyn PlayerRegistry {
        unimplemented!("mock")
    }
    fn player_registry_handle(&self) -> Arc<dyn PlayerRegistry> {
        unimplemented!("mock")
    }
    fn server_manager(&self) -> &dyn ServerManager {
        unimplemented!("mock")
    }
    fn server_manager_handle(&self) -> Arc<dyn ServerManager> {
        unimplemented!("mock")
    }
    fn ban_service(&self) -> &dyn BanService {
        unimplemented!("mock")
    }
    fn ban_service_handle(&self) -> Arc<dyn BanService> {
        unimplemented!("mock")
    }
    fn register_ban_provider(
        &self,
        _provider: Arc<dyn BanProvider>,
    ) -> Result<(), BanProviderRejected> {
        unimplemented!("mock")
    }
    fn register_permission_provider(
        &self,
        _provider: Arc<dyn PermissionProvider>,
    ) -> Result<(), PermissionProviderRejected> {
        unimplemented!("mock")
    }
    fn register_permission_node(&self, _node: PermissionNode) -> Result<(), PermissionNodeError> {
        unimplemented!("mock")
    }
    fn permission_nodes(&self) -> Vec<PermissionNodeInfo> {
        unimplemented!("mock")
    }
    fn config_service(&self) -> &dyn ConfigService {
        unimplemented!("mock")
    }
    fn config_service_handle(&self) -> Arc<dyn ConfigService> {
        unimplemented!("mock")
    }
    fn load_balancer_service(&self) -> &dyn LoadBalancerService {
        unimplemented!("mock")
    }
    fn load_balancer_service_handle(&self) -> Arc<dyn LoadBalancerService> {
        unimplemented!("mock")
    }
    fn command_manager(&self) -> &dyn CommandManager {
        unimplemented!("mock")
    }
    fn command_manager_handle(&self) -> Arc<dyn CommandManager> {
        unimplemented!("mock")
    }
    fn scheduler(&self) -> &dyn Scheduler {
        unimplemented!("mock")
    }
    fn scheduler_handle(&self) -> Arc<dyn Scheduler> {
        unimplemented!("mock")
    }
    fn services(&self) -> &dyn ServiceRegistry {
        unimplemented!("mock")
    }
    fn services_handle(&self) -> Arc<dyn ServiceRegistry> {
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
    fn plugin_registry(&self) -> &dyn PluginRegistry {
        unimplemented!("mock")
    }
    fn plugin_registry_handle(&self) -> Arc<dyn PluginRegistry> {
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
}

pub struct MockPluginContextFactory;

impl PluginContextFactory for MockPluginContextFactory {
    fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
        Arc::new(MockPluginContext::new(plugin_id))
    }
}
