//! Timeout and keepalive configuration.

use std::time::Duration;

use serde::Deserialize;

use crate::defaults;

/// Server-specific timeouts (overrides global settings).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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

/// TCP keepalive configuration.
///
/// Controls the keepalive probes sent on TCP connections
/// to detect dead connections.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
