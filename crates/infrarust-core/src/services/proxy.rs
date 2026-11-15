//! [`ProxyServices`] — aggregates shared services passed to connection handlers.

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
use crate::limbo::registry_cache::RegistryCodecCache;
use crate::loadbalancer::BackendHealthView;
use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::plugin_messaging::PluginMessaging;
use crate::provider::registry::PluginRoutes;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;
use crate::services::command_manager::CommandManagerImpl;

/// Shared services passed to connection handlers.
///
/// Created once in [`ProxyServer::new()`](crate::server::ProxyServer::new)
/// and cloned (all fields are `Arc`) for each connection. This replaces
/// passing 9+ individual parameters to handlers.
#[derive(Clone)]
pub struct ProxyServices {
    /// The event bus for dispatching lifecycle and packet events.
    pub event_bus: Arc<EventBusImpl>,
    /// Player registry for looking up connected players.
    pub player_registry: Arc<PlayerRegistryImpl>,
    /// Command manager for registering and dispatching `/` commands.
    pub command_manager: Arc<CommandManagerImpl>,
    /// Connection registry for tracking active player sessions.
    pub connection_registry: Arc<ConnectionRegistry>,
    /// Per-address session counts, handed to every `PlayerSession`.
    pub backend_load: Arc<crate::loadbalancer::BackendLoad>,
    /// Counts as backend selection sees them: attached sessions plus logins
    /// still negotiating. Used for selection outside the login pipeline.
    pub pending_backends: Arc<crate::loadbalancer::PendingRegistry>,
    /// Packet registry for decoding/encoding packets by version.
    pub packet_registry: Arc<PacketRegistry>,
    /// Server manager for starting/stopping managed servers.
    pub server_manager: Option<Arc<ServerManagerService>>,
    /// Ban manager for checking and issuing bans.
    pub ban_manager: Arc<BanManager>,
    /// Proxy configuration.
    pub config: Arc<ProxyConfig>,
    /// Path of the `infrarust.toml` the proxy was started from.
    pub config_path: std::path::PathBuf,
    /// Domain router for resolving server configs by domain.
    pub domain_router: Arc<DomainRouter>,
    /// Per-address backend health, consumed for backend selection outside
    /// the login pipeline (server switches, limbo exits).
    pub backend_health: Arc<dyn BackendHealthView>,
    /// Backend visibility and drain control, handed to plugins.
    pub load_balancer_service: Arc<crate::services::load_balancer_service::LoadBalancerServiceImpl>,
    /// Codec filter registry for building per-connection filter chains.
    pub codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    /// Transport filter registry whose chain is applied to accepted connections.
    pub transport_filter_registry: Arc<TransportFilterRegistryImpl>,
    pub connection_claims: Arc<ConnectionClaims>,
    /// Registry of limbo handler instances, keyed by name.
    pub limbo_handler_registry: Arc<LimboHandlerRegistry>,
    /// Multi-version registry data cache for limbo login.
    pub registry_codec_cache: Arc<RegistryCodecCache>,
    pub plugin_routes: PluginRoutes,
    pub forwarding_mode: Arc<ForwardingMode>,
    pub forwarding_secret: Option<Arc<[u8]>>,
    pub permission_service: Arc<PermissionService>,
    pub plugin_messaging: Arc<PluginMessaging>,
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
