//! Configuration service.

use crate::types::ServerId;

pub mod private {
    /// Sealed — only the proxy implements [`ConfigService`](super::ConfigService).
    pub trait Sealed {}
}

/// The proxy mode for a server connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProxyMode {
    /// Raw TCP forwarding — proxy cannot inspect or inject packets.
    Passthrough,
    /// Zero-copy forwarding — similar to Passthrough but with optimizations.
    ZeroCopy,
    /// Proxy terminates the client connection and re-encodes packets.
    ClientOnly,
    /// Offline mode — no Mojang authentication.
    Offline,
    /// Full server-side integration.
    ServerOnly,
}

/// Configuration for a backend server.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ServerConfig {
    /// The server's unique identifier.
    pub id: ServerId,
    /// Network this server belongs to. Only servers in the same network
    /// can switch between each other. `None` = isolated.
    pub network: Option<String>,
    /// Network addresses for this server.
    pub addresses: Vec<crate::types::ServerAddress>,
    /// Domain names that route to this server.
    pub domains: Vec<String>,
    /// The proxy mode for connections to this server.
    pub proxy_mode: ProxyMode,
    /// Ordered list of limbo handler names to apply.
    pub limbo_handlers: Vec<String>,
    /// Maximum number of players (0 = unlimited).
    pub max_players: u32,
    /// Disconnect message sent when the backend is unreachable.
    pub disconnect_message: Option<String>,
    /// Whether PROXY protocol is sent to the backend.
    pub send_proxy_protocol: bool,
    /// Whether this server has a server manager configured (auto start/stop).
    pub has_server_manager: bool,
}

impl ServerConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ServerId,
        network: Option<String>,
        addresses: Vec<crate::types::ServerAddress>,
        domains: Vec<String>,
        proxy_mode: ProxyMode,
        limbo_handlers: Vec<String>,
        max_players: u32,
        disconnect_message: Option<String>,
        send_proxy_protocol: bool,
        has_server_manager: bool,
    ) -> Self {
        Self {
            id,
            network,
            addresses,
            domains,
            proxy_mode,
            limbo_handlers,
            max_players,
            disconnect_message,
            send_proxy_protocol,
            has_server_manager,
        }
    }
}

/// Read-only access to proxy configuration.
///
/// Obtained via [`PluginContext::config_service()`](crate::plugin::PluginContext::config_service).
pub trait ConfigService: Send + Sync + private::Sealed {
    /// Returns the configuration for a specific server.
    fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig>;

    /// Returns all server configurations.
    fn get_all_server_configs(&self) -> Vec<ServerConfig>;

    /// Returns a configuration value by key, or `None` if not set.
    fn get_value(&self, key: &str) -> Option<String>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn proxy_mode_non_exhaustive() {
        let mode = ProxyMode::Passthrough;
        #[allow(unreachable_patterns)]
        match mode {
            ProxyMode::Passthrough
            | ProxyMode::ZeroCopy
            | ProxyMode::ClientOnly
            | ProxyMode::Offline
            | ProxyMode::ServerOnly
            | _ => {}
        }
    }
}
