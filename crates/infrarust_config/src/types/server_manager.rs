//! Server manager configuration (auto start/stop).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::defaults;

/// Server manager configuration (auto start/stop).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerManagerConfig {
    Local(LocalManagerConfig),
    Pterodactyl(PterodactylManagerConfig),
    Crafty(CraftyManagerConfig),
}

/// Local provider: launches a local Java process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
