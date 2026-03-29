//! Read-only proxy information exposed to plugins.
//!
//! Contains only non-sensitive configuration values and version info.

use std::net::SocketAddr;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ProxyInfo {
    /// Infrarust version (from `CARGO_PKG_VERSION`).
    pub version: String,

    /// Listen address (e.g. `0.0.0.0:25565`).
    pub bind: SocketAddr,

    /// Maximum simultaneous connections (0 = unlimited).
    pub max_connections: u32,

    /// Backend connection timeout.
    pub connect_timeout: Duration,

    /// Whether the proxy accepts HAProxy PROXY protocol.
    pub receive_proxy_protocol: bool,

    /// Number of tokio worker threads (0 = auto).
    pub worker_threads: usize,

    /// Enables `SO_REUSEPORT` (Linux only).
    pub so_reuseport: bool,

    /// Global rate limiting settings.
    pub rate_limit: RateLimitInfo,

    /// Status ping cache settings.
    pub status_cache: StatusCacheInfo,

    /// TCP keepalive settings.
    pub keepalive: KeepaliveInfo,

    /// Whether telemetry (OpenTelemetry) is enabled.
    pub telemetry_enabled: bool,

    /// Whether a Docker provider is configured.
    pub docker_enabled: bool,

    /// Whether the web admin API is enabled.
    pub web_api_enabled: bool,

    /// Whether the web admin UI is enabled.
    pub web_ui_enabled: bool,

    /// Behavior when a client connects with an unknown domain.
    pub unknown_domain_behavior: UnknownDomainBehavior,
}

/// Rate limiting configuration (read-only).
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// Maximum login connections per IP per window.
    pub max_connections: u32,
    /// Window duration for logins.
    pub window: Duration,
    /// Separate limit for status pings.
    pub status_max: u32,
    /// Window duration for status pings.
    pub status_window: Duration,
}

/// Status cache configuration (read-only).
#[derive(Debug, Clone)]
pub struct StatusCacheInfo {
    /// Time-to-live for a cache entry.
    pub ttl: Duration,
    /// Maximum number of entries.
    pub max_entries: usize,
}

#[derive(Debug, Clone)]
pub struct KeepaliveInfo {
    /// Idle duration before the first probe.
    pub time: Duration,
    /// Interval between probes.
    pub interval: Duration,
    /// Number of failed probes before closing.
    pub retries: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnknownDomainBehavior {
    DefaultMotd,
    Drop,
}

impl Default for ProxyInfo {
    fn default() -> Self {
        Self {
            version: String::new(),
            bind: std::net::SocketAddr::from(([0, 0, 0, 0], 25565)),
            max_connections: 0,
            connect_timeout: Duration::from_secs(5),
            receive_proxy_protocol: false,
            worker_threads: 0,
            so_reuseport: false,
            rate_limit: RateLimitInfo {
                max_connections: 0,
                window: Duration::from_secs(60),
                status_max: 0,
                status_window: Duration::from_secs(60),
            },
            status_cache: StatusCacheInfo {
                ttl: Duration::from_secs(30),
                max_entries: 1000,
            },
            keepalive: KeepaliveInfo {
                time: Duration::from_secs(30),
                interval: Duration::from_secs(10),
                retries: 3,
            },
            telemetry_enabled: false,
            docker_enabled: false,
            web_api_enabled: false,
            web_ui_enabled: false,
            unknown_domain_behavior: UnknownDomainBehavior::DefaultMotd,
        }
    }
}
