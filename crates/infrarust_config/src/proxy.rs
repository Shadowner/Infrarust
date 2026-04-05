//! Global proxy configuration (`infrarust.toml`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::defaults;
use crate::types::{
    BanConfig, DockerProviderConfig, ForwardingConfig, IpFilterConfig, KeepaliveConfig, MotdConfig,
    PermissionsConfig, RateLimitConfig, StatusCacheConfig, TelemetryConfig, WebConfig,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownDomainBehavior {
    #[default]
    DefaultMotd,
    Drop,
}

/// Corresponds to the `infrarust.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    /// Listen address, e.g., "0.0.0.0:25565"
    #[serde(default = "defaults::bind")]
    pub bind: SocketAddr,

    /// Maximum number of simultaneous connections (0 = unlimited)
    #[serde(default)]
    pub max_connections: u32,

    /// Backend connection timeout
    #[serde(default = "defaults::connect_timeout")]
    #[serde(with = "humantime_serde")]
    pub connect_timeout: Duration,

    /// Enables receiving proxy protocol (`HAProxy` v1/v2)
    #[serde(default)]
    pub receive_proxy_protocol: bool,

    /// Path to the server configuration directory
    #[serde(default = "defaults::servers_dir")]
    pub servers_dir: PathBuf,

    #[serde(default = "defaults::plugins_dir")]
    pub plugins_dir: PathBuf,

    /// Number of tokio worker threads (0 = auto)
    #[serde(default)]
    pub worker_threads: usize,

    /// Global rate limiting configuration
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Status ping cache configuration
    #[serde(default)]
    pub status_cache: StatusCacheConfig,

    /// Default MOTD when no server matches
    #[serde(default)]
    pub default_motd: Option<MotdConfig>,

    /// Telemetry configuration (absent = disabled)
    #[serde(default)]
    pub telemetry: Option<TelemetryConfig>,

    /// TCP keepalive configuration
    #[serde(default)]
    pub keepalive: KeepaliveConfig,

    /// Enables `SO_REUSEPORT` (Linux only)
    #[serde(default)]
    pub so_reuseport: bool,

    /// Ban system configuration
    #[serde(default)]
    pub ban: BanConfig,

    /// Docker provider configuration (optional).
    /// Present in the TOML even without the `docker` feature compiled.
    #[serde(default)]
    pub docker: Option<DockerProviderConfig>,

    #[serde(default)]
    pub ip_filter: Option<IpFilterConfig>,

    #[serde(default)]
    pub unknown_domain_behavior: UnknownDomainBehavior,

    #[serde(default = "defaults::announce_proxy_commands")]
    pub announce_proxy_commands: bool,

    #[serde(default)]
    pub forwarding: Option<ForwardingConfig>,

    /// Web admin API / UI configuration (absent = web plugin not loaded).
    #[serde(default)]
    pub web: Option<WebConfig>,

    #[serde(default)]
    pub permissions: PermissionsConfig,

    /// Plugin configurations keyed by plugin ID.
    #[serde(default)]
    pub plugins: HashMap<String, PluginConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// Path to the plugin binary/library.
    #[serde(default)]
    pub path: Option<String>,

    /// Permissions granted to this plugin.
    #[serde(default)]
    pub permissions: Vec<String>,

    /// Whether the plugin is enabled (default: true).
    #[serde(default)]
    pub enabled: Option<bool>,
}
