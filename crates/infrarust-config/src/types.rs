//! Fundamental types: enums, value objects, and shared configuration structs.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use ipnet::IpNet;
use serde::Deserialize;

use crate::defaults;
use crate::error::ConfigError;

/// Default Minecraft port.
pub const DEFAULT_MC_PORT: u16 = 25565;

// ─────────────────────────── Proxy Mode ───────────────────────────

/// Supported proxy modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProxyMode {
    /// Raw forwarding via `tokio::io::copy_bidirectional`.
    #[default]
    Passthrough,
    /// Raw forwarding via `splice(2)` on Linux.
    ZeroCopy,
    /// Mojang auth on the proxy side, backend in `online_mode=false`.
    ClientOnly,
    /// No authentication, transparent relay.
    Offline,
    /// Authentication handled by the backend.
    ServerOnly,
    /// Encryption on both sides (new in V2).
    Full,
}

impl ProxyMode {
    /// Returns `true` if the proxy parses packets beyond the handshake.
    pub const fn is_intercepted(&self) -> bool {
        matches!(self, Self::ClientOnly | Self::Offline | Self::Full)
    }

    /// Returns `true` if the proxy performs raw forwarding after the handshake.
    pub const fn is_forwarding(&self) -> bool {
        matches!(self, Self::Passthrough | Self::ZeroCopy | Self::ServerOnly)
    }
}

// ─────────────────────────── Server Address ───────────────────────

/// Address of a backend server.
///
/// Deserializes from a string `"host:port"` or `"host"` (default port = 25565).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl FromStr for ServerAddress {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Try to parse as SocketAddr first (IP:port)
        if let Ok(sock) = s.parse::<SocketAddr>() {
            return Ok(Self {
                host: sock.ip().to_string(),
                port: sock.port(),
            });
        }

        // Otherwise, split on the last ':'
        if let Some((host, port_str)) = s.rsplit_once(':')
            && let Ok(port) = port_str.parse::<u16>()
        {
            return Ok(Self {
                host: host.to_string(),
                port,
            });
        }

        // No port → default 25565
        if s.is_empty() {
            return Err(ConfigError::InvalidAddress(s.to_string()));
        }

        Ok(Self {
            host: s.to_string(),
            port: DEFAULT_MC_PORT,
        })
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// Serde deserialization from a string.
impl<'de> Deserialize<'de> for ServerAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ─────────────────────────── Domain Rewrite ───────────────────────

/// How to rewrite the domain in the Minecraft handshake
/// before forwarding it to the backend.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DomainRewrite {
    /// No rewrite — the original domain is forwarded as-is.
    #[default]
    None,
    /// Rewrites with an explicit domain.
    Explicit(String),
    /// Extracts the domain from the first backend address.
    FromBackend,
}

// ─────────────────────────── Rate Limit ───────────────────────────

/// Rate limiting configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitConfig {
    /// Maximum login connections per IP per window.
    #[serde(default = "defaults::rate_limit_max")]
    pub max_connections: u32,

    /// Window duration for logins.
    #[serde(default = "defaults::rate_limit_window")]
    #[serde(with = "humantime_serde")]
    pub window: Duration,

    /// Separate limit for status pings (more permissive).
    #[serde(default = "defaults::rate_limit_status_max")]
    pub status_max: u32,

    /// Window duration for status pings.
    #[serde(default = "defaults::rate_limit_status_window")]
    #[serde(with = "humantime_serde")]
    pub status_window: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_connections: defaults::rate_limit_max(),
            window: defaults::rate_limit_window(),
            status_max: defaults::rate_limit_status_max(),
            status_window: defaults::rate_limit_status_window(),
        }
    }
}

// ─────────────────────────── Status Cache ─────────────────────────

/// Status ping cache configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusCacheConfig {
    /// Time-to-live for a cache entry.
    #[serde(default = "defaults::status_cache_ttl")]
    #[serde(with = "humantime_serde")]
    pub ttl: Duration,

    /// Maximum number of entries.
    #[serde(default = "defaults::status_cache_max_entries")]
    pub max_entries: usize,
}

impl Default for StatusCacheConfig {
    fn default() -> Self {
        Self {
            ttl: defaults::status_cache_ttl(),
            max_entries: defaults::status_cache_max_entries(),
        }
    }
}

// ─────────────────────────── MOTD ─────────────────────────────────

/// MOTD per server state.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotdConfig {
    pub online: Option<MotdEntry>,
    pub offline: Option<MotdEntry>,
    pub sleeping: Option<MotdEntry>,
    pub starting: Option<MotdEntry>,
    pub crashed: Option<MotdEntry>,
    pub stopping: Option<MotdEntry>,
    pub unreachable: Option<MotdEntry>,
}

/// A MOTD entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotdEntry {
    /// MOTD text (supports Minecraft formatting codes).
    pub text: String,
    /// Path to the favicon (PNG), base64 string, or URL.
    #[serde(default)]
    pub favicon: Option<String>,
    /// Version displayed in the client.
    #[serde(default)]
    pub version_name: Option<String>,
    /// Maximum player count displayed.
    #[serde(default)]
    pub max_players: Option<u32>,
}

// ─────────────────────────── Timeouts ─────────────────────────────

/// Server-specific timeouts (overrides global settings).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeoutConfig {
    #[serde(default = "defaults::connect_timeout")]
    #[serde(with = "humantime_serde")]
    pub connect: Duration,

    #[serde(default = "defaults::read_timeout")]
    #[serde(with = "humantime_serde")]
    pub read: Duration,

    #[serde(default = "defaults::write_timeout")]
    #[serde(with = "humantime_serde")]
    pub write: Duration,
}

// ─────────────────────────── Keepalive ────────────────────────────

/// TCP keepalive configuration.
///
/// Controls the keepalive probes sent on TCP connections
/// to detect dead connections.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeepaliveConfig {
    /// Idle duration before the first probe.
    #[serde(default = "defaults::keepalive_time")]
    #[serde(with = "humantime_serde")]
    pub time: Duration,

    /// Interval between probes.
    #[serde(default = "defaults::keepalive_interval")]
    #[serde(with = "humantime_serde")]
    pub interval: Duration,

    /// Number of failed probes before closing the connection.
    #[serde(default = "defaults::keepalive_retries")]
    pub retries: u32,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            time: defaults::keepalive_time(),
            interval: defaults::keepalive_interval(),
            retries: defaults::keepalive_retries(),
        }
    }
}

// ─────────────────────────── IP Filter ────────────────────────────

/// IP filtering by CIDR.
///
/// If `whitelist` is non-empty, only IPs in the whitelist are allowed.
/// If `blacklist` is non-empty, IPs in the blacklist are rejected.
/// The whitelist is evaluated first.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpFilterConfig {
    #[serde(default)]
    pub whitelist: Vec<IpNet>,
    #[serde(default)]
    pub blacklist: Vec<IpNet>,
}

impl IpFilterConfig {
    /// Checks whether an IP is allowed by this filter.
    pub fn is_allowed(&self, ip: &std::net::IpAddr) -> bool {
        if !self.whitelist.is_empty() {
            return self.whitelist.iter().any(|net| net.contains(ip));
        }
        if !self.blacklist.is_empty() {
            return !self.blacklist.iter().any(|net| net.contains(ip));
        }
        true
    }
}

// ─────────────────────────── Server Manager ───────────────────────

/// Server manager configuration (auto start/stop).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerManagerConfig {
    Local(LocalManagerConfig),
    Pterodactyl(PterodactylManagerConfig),
    Crafty(CraftyManagerConfig),
}

/// Local provider: launches a local Java process.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalManagerConfig {
    /// Command to execute (e.g., "java")
    pub command: String,
    /// Working directory
    pub working_dir: std::path::PathBuf,
    /// Arguments (ex: `["-Xmx4G", "-jar", "server.jar", "nogui"]`)
    #[serde(default)]
    pub args: Vec<String>,
    /// Pattern in stdout indicating the server is ready
    #[serde(default = "defaults::ready_pattern")]
    pub ready_pattern: String,
    /// Timeout for graceful shutdown
    #[serde(default = "defaults::shutdown_timeout")]
    #[serde(with = "humantime_serde")]
    pub shutdown_timeout: Duration,
    /// Idle duration before automatic shutdown (None = disabled)
    #[serde(default)]
    #[serde(with = "humantime_serde")]
    pub shutdown_after: Option<Duration>,
    /// Timeout for server startup
    #[serde(default = "defaults::start_timeout")]
    #[serde(with = "humantime_serde")]
    pub start_timeout: Duration,
}

/// Pterodactyl provider: REST API.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PterodactylManagerConfig {
    pub api_url: String,
    pub api_key: String,
    pub server_id: String,
    /// Idle duration before automatic shutdown (None = disabled)
    #[serde(default)]
    #[serde(with = "humantime_serde")]
    pub shutdown_after: Option<Duration>,
    /// Timeout for server startup
    #[serde(default = "defaults::start_timeout")]
    #[serde(with = "humantime_serde")]
    pub start_timeout: Duration,
    /// Polling interval to check server state
    #[serde(default = "defaults::poll_interval")]
    #[serde(with = "humantime_serde")]
    pub poll_interval: Duration,
}

/// Crafty Controller provider: REST API.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CraftyManagerConfig {
    pub api_url: String,
    pub api_key: String,
    pub server_id: String,
    /// Idle duration before automatic shutdown (None = disabled)
    #[serde(default)]
    #[serde(with = "humantime_serde")]
    pub shutdown_after: Option<Duration>,
    /// Timeout for server startup
    #[serde(default = "defaults::start_timeout")]
    #[serde(with = "humantime_serde")]
    pub start_timeout: Duration,
    /// Polling interval to check server state
    #[serde(default = "defaults::poll_interval")]
    #[serde(with = "humantime_serde")]
    pub poll_interval: Duration,
}

// ─────────────────────────── Telemetry ────────────────────────────

/// OpenTelemetry telemetry configuration.
///
/// Sub-sections: `[telemetry.metrics]`, `[telemetry.traces]`, `[telemetry.resource]`.
/// Absent from the TOML file means `None` in `ProxyConfig` (no telemetry).
#[derive(Debug, Clone, Deserialize)]
pub struct TelemetryConfig {
    /// Enables telemetry. `false` = initialized but no export.
    #[serde(default)]
    pub enabled: bool,

    /// Endpoint OTLP (ex: "<http://localhost:4317>"). `None` = SDK default.
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Export protocol: "grpc" or "http".
    #[serde(default = "defaults::telemetry_protocol")]
    pub protocol: String,

    /// Metrics configuration.
    #[serde(default)]
    pub metrics: MetricsConfig,

    /// Traces configuration.
    #[serde(default)]
    pub traces: TracesConfig,

    /// `OTel` resource attributes.
    #[serde(default)]
    pub resource: ResourceConfig,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            protocol: defaults::telemetry_protocol(),
            metrics: MetricsConfig::default(),
            traces: TracesConfig::default(),
            resource: ResourceConfig::default(),
        }
    }
}

/// `OTel` metrics configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    /// Enables metrics export.
    #[serde(default = "defaults::true_val")]
    pub enabled: bool,

    /// Metrics export interval.
    #[serde(default = "defaults::metrics_export_interval")]
    #[serde(with = "humantime_serde")]
    pub export_interval: Duration,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::true_val(),
            export_interval: defaults::metrics_export_interval(),
        }
    }
}

/// `OTel` traces configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracesConfig {
    /// Enables traces export.
    #[serde(default = "defaults::true_val")]
    pub enabled: bool,

    /// Sampling ratio for status pings (0.0-1.0).
    /// Login connections are always traced at 100%.
    #[serde(default = "defaults::sampling_ratio")]
    pub sampling_ratio: f64,
}

impl Default for TracesConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::true_val(),
            sampling_ratio: defaults::sampling_ratio(),
        }
    }
}

/// `OTel` resource attributes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    /// `OTel` service name.
    #[serde(default = "defaults::service_name")]
    pub service_name: String,

    /// `OTel` service version.
    #[serde(default = "defaults::service_version")]
    pub service_version: String,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            service_name: defaults::service_name(),
            service_version: defaults::service_version(),
        }
    }
}

// ─────────────────────────── Ban ────────────────────────────────

/// Ban system configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BanConfig {
    /// Path to the JSON bans file.
    #[serde(default = "defaults::ban_file")]
    pub file: std::path::PathBuf,

    /// Automatic purge interval for expired bans.
    #[serde(default = "defaults::ban_purge_interval")]
    #[serde(with = "humantime_serde")]
    pub purge_interval: Duration,

    /// Enables the audit log (tracks ban/unban operations).
    #[serde(default = "defaults::ban_audit_log")]
    pub enable_audit_log: bool,
}

impl Default for BanConfig {
    fn default() -> Self {
        Self {
            file: defaults::ban_file(),
            purge_interval: defaults::ban_purge_interval(),
            enable_audit_log: defaults::ban_audit_log(),
        }
    }
}

// ─────────────────────────── Docker Provider ───────────────────────

/// Docker provider configuration.
///
/// This type is always compiled (no feature gate) so that
/// `ProxyConfig` can parse a `[docker]` section regardless of
/// the build configuration. The `DockerProvider` itself
/// is feature-gated in `infrarust-core`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerProviderConfig {
    /// Endpoint Docker (ex: "<unix:///var/run/docker.sock>").
    #[serde(default = "defaults::docker_endpoint")]
    pub endpoint: String,

    /// Preferred Docker network for address resolution.
    #[serde(default)]
    pub network: Option<String>,

    /// Fallback polling interval.
    #[serde(default = "defaults::docker_poll_interval")]
    #[serde(with = "humantime_serde")]
    pub poll_interval: Duration,

    /// Reconnection delay after Docker daemon disconnection.
    #[serde(default = "defaults::docker_reconnect_delay")]
    #[serde(with = "humantime_serde")]
    pub reconnect_delay: Duration,
}

impl Default for DockerProviderConfig {
    fn default() -> Self {
        Self {
            endpoint: defaults::docker_endpoint(),
            network: None,
            poll_interval: defaults::docker_poll_interval(),
            reconnect_delay: defaults::docker_reconnect_delay(),
        }
    }
}
