//! Backend server configuration (one per `.toml` file in `servers_dir`).

use serde::Deserialize;

use crate::types::{
    DomainRewrite, IpFilterConfig, MotdConfig, ProxyMode, ServerAddress, ServerManagerConfig,
    TimeoutConfig,
};

/// Configuration for a Minecraft backend server.
/// Each file in `servers_dir/` deserializes into this type.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Unique identifier. Derived from the filename if absent.
    #[serde(default)]
    pub id: Option<String>,

    /// Domains that route to this server.
    /// Supports wildcards: "*.mc.example.com"
    pub domains: Vec<String>,

    /// Backend addresses (host:port). Multiple = future load balancing.
    pub addresses: Vec<ServerAddress>,

    /// Proxy mode for this server
    #[serde(default)]
    pub proxy_mode: ProxyMode,

    /// Sends proxy protocol to the backend
    #[serde(default)]
    pub send_proxy_protocol: bool,

    /// Domain rewrite in the handshake
    #[serde(default)]
    pub domain_rewrite: DomainRewrite,

    /// MOTD per server state
    #[serde(default)]
    pub motd: MotdConfig,

    /// Automatic server management (start/stop)
    #[serde(default)]
    pub server_manager: Option<ServerManagerConfig>,

    /// Server-specific timeouts (overrides global settings)
    #[serde(default)]
    pub timeouts: Option<TimeoutConfig>,

    /// Maximum number of players (0 = unlimited)
    #[serde(default)]
    pub max_players: u32,

    /// Server-specific IP filters
    #[serde(default)]
    pub ip_filter: Option<IpFilterConfig>,

    /// Disconnect message sent to the player when the backend is unreachable.
    #[serde(default)]
    pub disconnect_message: Option<String>,

    /// Limbo handler chain for this server (plugin IDs, executed in order).
    #[serde(default)]
    pub limbo_handlers: Vec<String>,
}

impl ServerConfig {
    /// Returns the effective identifier for this config.
    ///
    /// If `id` is `None`, returns `"unknown"`. In practice the `FileProvider`
    /// sets `id` from the filename (without extension) before handing the
    /// config to the rest of the system.
    pub fn effective_id(&self) -> String {
        self.id.clone().unwrap_or_else(|| "unknown".to_string())
    }

    /// Returns the disconnect message for when the backend is unreachable.
    pub fn effective_disconnect_message(&self) -> &str {
        self.disconnect_message
            .as_deref()
            .unwrap_or("Server is currently unreachable. Please try again later.")
    }
}
