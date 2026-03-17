//! Rate limiting configuration.

use std::time::Duration;

use serde::Deserialize;

use crate::defaults;

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
