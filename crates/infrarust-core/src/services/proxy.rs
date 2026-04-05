//! [`ProxyServices`] — aggregates shared services passed to connection handlers.

use std::sync::Arc;

use infrarust_config::{ForwardingMode as ConfigForwardingMode, ProxyConfig, ServerConfig};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_server_manager::ServerManagerService;
use tokio::sync::mpsc;

use crate::ban::manager::BanManager;
use crate::event_bus::EventBusImpl;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_chain::TransportFilterChain;
use crate::forwarding::{ForwardingHandler, ForwardingMode, build_forwarding_handler};
use crate::limbo::registry::LimboHandlerRegistry;
use crate::limbo::registry_cache::RegistryCodecCache;
use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::provider::ProviderEvent;
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
    /// Codec filter registry for building per-connection filter chains.
    pub codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    /// Transport filter chain applied to accepted connections.
    pub transport_filter_chain: TransportFilterChain,
    /// Registry of limbo handler instances, keyed by name.
    pub limbo_handler_registry: Arc<LimboHandlerRegistry>,
    /// Multi-version registry data cache for limbo login.
    pub registry_codec_cache: Arc<RegistryCodecCache>,
    pub provider_event_sender: mpsc::Sender<ProviderEvent>,
    pub forwarding_mode: Arc<ForwardingMode>,
    pub forwarding_secret: Option<Arc<[u8]>>,
    pub permission_service: Arc<PermissionService>,
}

impl ProxyServices {
    pub fn resolve_forwarding_handler(&self, server_config: &ServerConfig) -> ForwardingHandler {
        if let Some(ref server_override) = server_config.forwarding_mode {
            let mode = config_to_core_mode(server_override, &self.forwarding_mode);
            build_forwarding_handler(&mode)
        } else {
            build_forwarding_handler(&self.forwarding_mode)
        }
    }

    pub fn forwarding_secret(&self) -> Option<&[u8]> {
        self.forwarding_secret.as_deref()
    }
}

fn config_to_core_mode(
    config_mode: &ConfigForwardingMode,
    global: &ForwardingMode,
) -> ForwardingMode {
    match config_mode {
        ConfigForwardingMode::None => ForwardingMode::None,
        ConfigForwardingMode::BungeeCord => {
            tracing::warn!(
                "A server overrides forwarding to BungeeCord legacy mode. \
                 This mode is fundamentally insecure — anyone reaching the backend \
                 directly can impersonate any player."
            );
            ForwardingMode::BungeeCord
        }
        ConfigForwardingMode::BungeeGuard => match global {
            ForwardingMode::BungeeGuard { token } => ForwardingMode::BungeeGuard {
                token: token.clone(),
            },
            ForwardingMode::Velocity { secret } => ForwardingMode::BungeeGuard {
                token: String::from_utf8_lossy(secret).to_string(),
            },
            _ => {
                tracing::error!(
                    "server overrides forwarding to bungeeguard but no secret is \
                     configured globally — falling back to no forwarding"
                );
                ForwardingMode::None
            }
        },
        ConfigForwardingMode::Velocity => match global {
            ForwardingMode::Velocity { secret } => ForwardingMode::Velocity {
                secret: secret.clone(),
            },
            _ => {
                tracing::error!(
                    "server overrides forwarding to velocity but no secret is \
                     configured globally — falling back to no forwarding"
                );
                ForwardingMode::None
            }
        },
    }
}
