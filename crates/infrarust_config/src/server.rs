//! Backend server configuration (one per `.toml` file in `servers_dir`).

use serde::{Deserialize, Serialize};

use crate::types::{
    DomainRewrite, ForwardingMode, IpFilterConfig, MotdConfig, ProxyMode, ServerAddress,
    ServerManagerConfig, TimeoutConfig,
};

/// Each file in `servers_dir/` deserializes into this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Unique identifier. Derived from the filename if absent.
    #[serde(default)]
    pub id: Option<String>,

    /// Human-readable server name. If set, becomes the effective ServerId
    /// (takes priority over `id`). Must match `[a-z0-9_-]+`.
    #[serde(default)]
    pub name: Option<String>,

    /// Network this server belongs to. Only servers in the same network
    /// can switch between each other. `None` = isolated, no switch allowed.
    #[serde(default)]
    pub network: Option<String>,

    /// Domains that route to this server.
    /// Supports wildcards: "*.mc.example.com"
    /// Empty = no domain routing (server accessible only via server switch).
    #[serde(default)]
    pub domains: Vec<String>,

    /// Backend addresses (host:port). Multiple = future load balancing.
    pub addresses: Vec<ServerAddress>,

    /// Proxy mode for this server
    #[serde(default)]
    pub proxy_mode: ProxyMode,

    #[serde(default)]
    pub forwarding_mode: Option<ForwardingMode>,

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
    /// Priority: `name` > `id` > `"unknown"`.
    /// If `name` is set, it becomes the server's identity (used as `ServerId`).
    /// Otherwise falls back to `id` (typically set by the provider from the filename).
    pub fn effective_id(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.id.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Returns the disconnect message for when the backend is unreachable.
    pub fn effective_disconnect_message(&self) -> &str {
        self.disconnect_message
            .as_deref()
            .unwrap_or("Server is currently unreachable. Please try again later.")
    }
}
