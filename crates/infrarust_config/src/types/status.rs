//! Status cache and MOTD configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Protocol version number. Set to -1 to show version name in red
    /// and display an X on the ping bars (server appears offline).
    #[serde(default)]
    pub version_protocol: Option<i32>,
    /// Maximum player count displayed.
    #[serde(default)]
    pub max_players: Option<u32>,
}
