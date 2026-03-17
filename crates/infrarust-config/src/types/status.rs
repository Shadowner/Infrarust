//! Status cache and MOTD configuration.

use std::time::Duration;

use serde::Deserialize;

use crate::defaults;

/// Status ping cache configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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

/// MOTD per server state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
