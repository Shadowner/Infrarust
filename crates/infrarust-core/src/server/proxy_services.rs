use std::sync::Arc;

use infrarust_config::{ProxyConfig, ServerConfig};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_server_manager::ServerManagerService;

use crate::ban::manager::BanManager;
use crate::event_bus::EventBusImpl;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::forwarding::server_override::resolve_forwarding_handler;
use crate::forwarding::{ForwardingError, ForwardingHandler, ForwardingMode};
use crate::handoff::ConnectionClaims;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::loadbalancer::BackendHealthView;
use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::plugin_messaging::PluginMessaging;
use crate::provider::registry::PluginRoutes;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;
use crate::services::command_manager::CommandManagerImpl;

#[derive(Clone)]
pub struct ProxyServices {
    pub event_bus: Arc<EventBusImpl>,
    pub player_registry: Arc<PlayerRegistryImpl>,
    pub command_manager: Arc<CommandManagerImpl>,
    pub connection_registry: Arc<ConnectionRegistry>,
    pub(crate) backend_load: Arc<crate::loadbalancer::BackendLoad>,
    pub(crate) pending_backends: Arc<crate::loadbalancer::PendingRegistry>,
    pub(crate) packet_registry: Arc<PacketRegistry>,
    pub server_manager: Option<Arc<ServerManagerService>>,
    pub ban_manager: Arc<BanManager>,
    pub(crate) config: Arc<ProxyConfig>,
    pub(crate) config_path: std::path::PathBuf,
    pub domain_router: Arc<DomainRouter>,
    pub backend_health: Arc<dyn BackendHealthView>,
    pub(crate) load_balancer_service:
        Arc<crate::services::load_balancer_service::LoadBalancerServiceImpl>,
    pub codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    pub transport_filter_registry: Arc<TransportFilterRegistryImpl>,
    pub connection_claims: Arc<ConnectionClaims>,
    pub(crate) limbo_handler_registry: Arc<LimboHandlerRegistry>,
    pub(crate) plugin_routes: PluginRoutes,
    pub(crate) forwarding_mode: Arc<ForwardingMode>,
    pub(crate) forwarding_secret: Option<Arc<[u8]>>,
    pub permission_service: Arc<PermissionService>,
    pub(crate) plugin_messaging: Arc<PluginMessaging>,
}

impl ProxyServices {
    pub fn resolve_forwarding_handler(
        &self,
        server_config: &ServerConfig,
    ) -> Result<ForwardingHandler, ForwardingError> {
        resolve_forwarding_handler(&self.forwarding_mode, server_config)
    }

    pub fn forwarding_secret(&self) -> Option<&[u8]> {
        self.forwarding_secret.as_deref()
    }
}
