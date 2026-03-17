//! [`ProxyServices`] — aggregates shared services passed to connection handlers.

use std::sync::Arc;

use infrarust_config::ProxyConfig;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_server_manager::ServerManagerService;

use crate::ban::manager::BanManager;
use crate::event_bus::EventBusImpl;
use crate::player::registry::PlayerRegistryImpl;
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
    /// Packet registry for decoding/encoding packets by version.
    pub packet_registry: Arc<PacketRegistry>,
    /// Server manager for starting/stopping managed servers.
    pub server_manager: Option<Arc<ServerManagerService>>,
    /// Ban manager for checking and issuing bans.
    pub ban_manager: Arc<BanManager>,
    /// Proxy configuration.
    pub config: Arc<ProxyConfig>,
    /// Domain router for resolving server configs by domain.
    pub domain_router: Arc<DomainRouter>,
}
