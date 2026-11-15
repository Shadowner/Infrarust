//! Global proxy configuration (`infrarust.toml`).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use ipnet::IpNet;
use serde::{Deserialize, Deserializer, Serialize};

use crate::defaults;
use crate::types::{
    ActiveHealthConfig, AuthConfig, BanConfig, DockerProviderConfig, EventsConfig,
    ForwardingConfig, IpFilterConfig, KeepaliveConfig, MotdConfig, PermissionsConfig,
    PluginMessagingConfig, PluginWasmConfig, RateLimitConfig, StatusCacheConfig, TelemetryConfig,
    WasmConfig, WebConfig,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownDomainBehavior {
    #[default]
    DefaultMotd,
    Drop,
}

fn deserialize_trusted_proxies<'de, D>(deserializer: D) -> Result<Vec<IpNet>, D::Error>
where
    D: Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer)?
        .into_iter()
        .map(|entry| {
            entry
                .parse::<IpNet>()
                .or_else(|_| entry.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| {
                    serde::de::Error::custom(format_args!(
                        "invalid trusted_proxies entry {entry:?}: expected an IP address such \
                         as 192.168.1.10 or a CIDR range such as 10.0.0.0/8"
                    ))
                })
        })
        .collect()
}

/// Corresponds to the `infrarust.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    /// Listen address, e.g., "0.0.0.0:25565"
    #[serde(default = "defaults::bind")]
    pub bind: SocketAddr,

    /// Maximum number of simultaneous connections (0 = unlimited)
    #[serde(default = "defaults::max_connections")]
    pub max_connections: u32,

    /// Maximum simultaneous connections per IPv4 address or IPv6 /64 (0 = unlimited)
    #[serde(default = "defaults::max_connections_per_ip")]
    pub max_connections_per_ip: u32,

    /// Backend connection timeout
    #[serde(default = "defaults::connect_timeout")]
    #[serde(with = "humantime_serde")]
    pub connect_timeout: Duration,

    /// Maximum backend addresses tried before giving up on a connection.
    /// Bounds worst-case login latency to `attempts × connect_timeout`.
    #[serde(default = "defaults::connect_max_attempts")]
    pub connect_max_attempts: usize,

    /// Enables receiving proxy protocol (`HAProxy` v1/v2)
    #[serde(default)]
    pub receive_proxy_protocol: bool,

    /// Peers allowed to send a proxy protocol header. Any other peer is closed
    /// without reading. Required when `receive_proxy_protocol` is enabled.
    #[serde(default, deserialize_with = "deserialize_trusted_proxies")]
    pub trusted_proxies: Vec<IpNet>,

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

    #[serde(default = "defaults::true_val")]
    pub announce_proxy_commands: bool,

    #[serde(default = "defaults::compression_threshold")]
    pub compression_threshold: i32,

    #[serde(default)]
    pub forwarding: Option<ForwardingConfig>,

    #[serde(default)]
    pub auth: AuthConfig,

    /// Web admin API / UI configuration (absent = web plugin not loaded).
    #[serde(default)]
    pub web: Option<WebConfig>,

    #[serde(default)]
    pub permissions: PermissionsConfig,

    /// Background probing that brings ejected backend addresses back.
    #[serde(default)]
    pub active_health: ActiveHealthConfig,

    #[serde(default)]
    pub events: EventsConfig,

    #[serde(default)]
    pub plugin_messaging: PluginMessagingConfig,

    #[serde(default)]
    pub wasm: WasmConfig,

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

    #[serde(default)]
    pub deny: Vec<String>,

    #[serde(default)]
    pub strict_capabilities: bool,

    #[serde(default)]
    pub wasm: Option<PluginWasmConfig>,

    /// Whether the plugin is enabled (default: true).
    #[serde(default = "defaults::true_val")]
    pub enabled: bool,
}
