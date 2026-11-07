//! [`ProxyServices`] — aggregates shared services passed to connection handlers.

use std::sync::Arc;

use infrarust_config::{ForwardingMode as ConfigForwardingMode, ProxyConfig, ServerConfig};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_server_manager::ServerManagerService;
use tokio::sync::mpsc;

use crate::ban::manager::BanManager;
use crate::event_bus::EventBusImpl;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::forwarding::{
    ForwardingError, ForwardingHandler, ForwardingMode, build_forwarding_handler,
};
use crate::handoff::ConnectionClaims;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::limbo::registry_cache::RegistryCodecCache;
use crate::loadbalancer::BackendHealthView;
use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::plugin_messaging::PluginMessaging;
use crate::provider::ProviderEvent;
use crate::routing::DomainRouter;
use crate::services::command_manager::CommandManagerImpl;
use crate::session::connection_registry::ConnectionRegistry;

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
    pub provider_event_sender: mpsc::Sender<ProviderEvent>,
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
        if let Some(ref server_override) = server_config.forwarding_mode {
            let mode = config_to_core_mode(server_override, &self.forwarding_mode)?;
            Ok(build_forwarding_handler(&mode))
        } else {
            Ok(build_forwarding_handler(&self.forwarding_mode))
        }
    }

    pub fn forwarding_secret(&self) -> Option<&[u8]> {
        self.forwarding_secret.as_deref()
    }
}

fn config_to_core_mode(
    config_mode: &ConfigForwardingMode,
    global: &ForwardingMode,
) -> Result<ForwardingMode, ForwardingError> {
    match (config_mode, global) {
        (ConfigForwardingMode::None, _) => Ok(ForwardingMode::None),
        (ConfigForwardingMode::BungeeCord, _) => {
            tracing::warn!(
                "A server overrides forwarding to BungeeCord legacy mode. \
                 This mode is fundamentally insecure — anyone reaching the backend \
                 directly can impersonate any player."
            );
            Ok(ForwardingMode::BungeeCord)
        }
        (ConfigForwardingMode::BungeeGuard, ForwardingMode::BungeeGuard { token }) => {
            Ok(ForwardingMode::BungeeGuard {
                token: token.clone(),
            })
        }
        (ConfigForwardingMode::Velocity, ForwardingMode::Velocity { secret }) => {
            Ok(ForwardingMode::Velocity {
                secret: secret.clone(),
            })
        }
        (
            ConfigForwardingMode::BungeeGuard,
            ForwardingMode::None | ForwardingMode::BungeeCord | ForwardingMode::Velocity { .. },
        ) => Err(ForwardingError::SecretMismatch {
            requested: "bungee_guard",
        }),
        (
            ConfigForwardingMode::Velocity,
            ForwardingMode::None | ForwardingMode::BungeeCord | ForwardingMode::BungeeGuard { .. },
        ) => Err(ForwardingError::SecretMismatch {
            requested: "velocity",
        }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn velocity() -> ForwardingMode {
        ForwardingMode::Velocity {
            secret: b"velocity-secret".to_vec(),
        }
    }

    fn bungee_guard() -> ForwardingMode {
        ForwardingMode::BungeeGuard {
            token: "token".to_string(),
        }
    }

    #[test]
    fn a_velocity_secret_never_becomes_a_bungee_guard_token() {
        assert!(matches!(
            config_to_core_mode(&ConfigForwardingMode::BungeeGuard, &velocity()),
            Err(ForwardingError::SecretMismatch {
                requested: "bungee_guard"
            })
        ));
    }

    #[test]
    fn an_override_without_its_secret_is_refused_instead_of_dropping_forwarding() {
        for (requested, global) in [
            (ConfigForwardingMode::BungeeGuard, ForwardingMode::None),
            (
                ConfigForwardingMode::BungeeGuard,
                ForwardingMode::BungeeCord,
            ),
            (ConfigForwardingMode::Velocity, ForwardingMode::None),
            (ConfigForwardingMode::Velocity, ForwardingMode::BungeeCord),
            (ConfigForwardingMode::Velocity, bungee_guard()),
        ] {
            assert!(
                matches!(
                    config_to_core_mode(&requested, &global),
                    Err(ForwardingError::SecretMismatch { .. })
                ),
                "{requested:?}"
            );
        }
    }

    #[test]
    fn an_override_keeps_the_secret_of_its_own_mode() {
        assert_eq!(
            config_to_core_mode(&ConfigForwardingMode::Velocity, &velocity()).unwrap(),
            velocity()
        );
        assert_eq!(
            config_to_core_mode(&ConfigForwardingMode::BungeeGuard, &bungee_guard()).unwrap(),
            bungee_guard()
        );
    }

    #[test]
    fn overrides_without_a_secret_work_under_any_global_mode() {
        for global in [
            ForwardingMode::None,
            ForwardingMode::BungeeCord,
            bungee_guard(),
            velocity(),
        ] {
            assert_eq!(
                config_to_core_mode(&ConfigForwardingMode::None, &global).unwrap(),
                ForwardingMode::None
            );
            assert_eq!(
                config_to_core_mode(&ConfigForwardingMode::BungeeCord, &global).unwrap(),
                ForwardingMode::BungeeCord
            );
        }
    }
}
