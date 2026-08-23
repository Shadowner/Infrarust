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

/// Where a server configuration came from.
#[derive(Debug, Clone)]
pub struct ServerSource {
    /// The server's identifier.
    pub id: String,
    /// Canonical provider id, e.g. `file@survival.toml`.
    pub provider_id: String,
    /// Provider type, e.g. `file`, `docker` or `plugin:admin_api:api`.
    pub provider_type: String,
    /// `true` when a plugin provider supplied this config, so the plugin
    /// owning it can rewrite it. Configs read from `servers_dir` or from
    /// Docker are never editable through a plugin.
    pub editable: bool,
}

/// Why a proxy configuration write was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigWriteError {
    /// The plugin does not hold [`Capability::ConfigWrite`](crate::permissions::Capability::ConfigWrite).
    #[error("the plugin is not allowed to write the proxy configuration")]
    PermissionDenied,
    /// The submitted document is not valid TOML, or does not match the schema.
    #[error("invalid configuration document: {0}")]
    Parse(String),
    /// The document parses but describes a configuration the proxy refuses.
    #[error("invalid configuration: {0}")]
    Validation(String),
    /// The document is fine but could not be persisted.
    #[error("cannot write the configuration file: {0}")]
    Io(String),
}

/// Access to proxy configuration.
///
/// Obtained via [`PluginContext::config_service()`](crate::plugin::PluginContext::config_service).
/// Everything but [`write_proxy_config_document`](Self::write_proxy_config_document)
/// is readable by any plugin.
pub trait ConfigService: Send + Sync + private::Sealed {
    /// Returns the configuration for a specific server.
    fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig>;

    /// Returns all server configurations.
    fn get_all_server_configs(&self) -> Vec<ServerConfig>;

    /// Returns the complete TOML document for a server, whatever provider
    /// supplied it, with every secret field redacted. Unlike
    /// [`get_server_config`](Self::get_server_config) this loses no fields.
    fn get_server_document(&self, server: &ServerId) -> Option<String>;

    /// Returns the provenance of every known server configuration.
    fn list_server_sources(&self) -> Vec<ServerSource>;

    /// Returns the global proxy configuration file as a TOML document, with
    /// every secret field redacted. This is the document a write edits, so it
    /// carries the file's own values, not the ones the proxy was started with.
    fn get_proxy_config_document(&self) -> String;

    /// Returns the global proxy configuration the proxy is actually running
    /// on — the file with its CLI overrides applied and its defaults filled
    /// in — as a TOML document with every secret field redacted.
    fn get_effective_proxy_config_document(&self) -> String;

    /// Replaces the global proxy configuration file with `toml`.
    ///
    /// Secret fields the document leaves out or carries redacted keep the
    /// value already on disk, so a document read back from
    /// [`get_proxy_config_document`](Self::get_proxy_config_document) can be
    /// submitted unchanged. Nothing is applied to the running proxy: the new
    /// configuration takes effect on restart.
    ///
    /// # Errors
    /// See [`ConfigWriteError`].
    fn write_proxy_config_document(&self, toml: &str) -> Result<(), ConfigWriteError>;

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
