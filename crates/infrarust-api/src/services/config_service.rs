//! Configuration service.

use crate::types::ServerId;

mod private {
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
pub struct ServerConfig {
    /// The server's unique identifier.
    pub id: ServerId,
    /// Network addresses for this server.
    pub addresses: Vec<crate::types::ServerAddress>,
    /// Domain names that route to this server.
    pub domains: Vec<String>,
    /// The proxy mode for connections to this server.
    pub proxy_mode: ProxyMode,
    /// Ordered list of limbo handler names to apply.
    pub limbo_handlers: Vec<String>,
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
