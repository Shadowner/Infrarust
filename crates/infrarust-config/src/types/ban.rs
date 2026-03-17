//! Ban system configuration.

use std::time::Duration;

use serde::Deserialize;

use crate::defaults;

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
