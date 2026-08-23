//! Active health probing configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// What an active probe actually does against a backend address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    /// TCP connect then close. Mirrors exactly what the proxy does when it
    /// opens a backend connection, so it cannot disagree with reality.
    #[default]
    Tcp,
    /// Full Minecraft status exchange. Slower and reaches the server's main
    /// thread on some implementations, but proves the server actually answers.
    StatusPing,
}

/// Background probing of backend addresses.
///
/// Recovery probing is what brings an ejected address back: without it, an
/// address only gets retried once every other address of its server has
/// failed. Proactive probing of healthy addresses is opt-in, since passive
/// health already covers the healthy-to-failing direction from real traffic.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveHealthConfig {
    #[serde(default = "defaults::enabled")]
    pub enabled: bool,

    #[serde(default)]
    pub kind: ProbeKind,

    /// How often ejected addresses are checked for probe eligibility.
    #[serde(default = "defaults::unhealthy_interval", with = "humantime_serde")]
    pub unhealthy_interval: Duration,

    /// Also probe addresses that are currently healthy.
    #[serde(default)]
    pub probe_healthy: bool,

    /// Sweep interval when `probe_healthy` is set.
    #[serde(default = "defaults::interval", with = "humantime_serde")]
    pub interval: Duration,

    #[serde(default = "defaults::timeout", with = "humantime_serde")]
    pub timeout: Duration,

    #[serde(default = "defaults::max_concurrent")]
    pub max_concurrent: usize,
}

mod defaults {
    use std::time::Duration;

    pub const fn enabled() -> bool {
        true
    }
    pub const fn unhealthy_interval() -> Duration {
        Duration::from_secs(10)
    }
    pub const fn interval() -> Duration {
        Duration::from_secs(30)
    }
    pub const fn timeout() -> Duration {
        Duration::from_secs(3)
    }
    pub const fn max_concurrent() -> usize {
        8
    }
}

impl Default for ActiveHealthConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::enabled(),
            kind: ProbeKind::default(),
            unhealthy_interval: defaults::unhealthy_interval(),
            probe_healthy: false,
            interval: defaults::interval(),
            timeout: defaults::timeout(),
            max_concurrent: defaults::max_concurrent(),
        }
    }
}
