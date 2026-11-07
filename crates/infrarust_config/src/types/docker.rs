//! Docker provider configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::defaults;

/// This type is always compiled (no feature gate) so that
/// `ProxyConfig` can parse a `[docker]` section regardless of
/// the build configuration. The `DockerProvider` itself
/// is feature-gated in `infrarust-core`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Domains containers may claim through `infrarust.domains`. Entries accept
    /// `*` and `?`. A container may always use `<container>.docker.local`.
    #[serde(default)]
    pub allowed_domains: Vec<String>,

    /// Lets containers inherit the proxy-wide `[forwarding]` mode, which hands
    /// them the forwarding secret. Otherwise they get `forwarding_mode = "none"`.
    #[serde(default)]
    pub inherit_forwarding: bool,
}

impl Default for DockerProviderConfig {
    fn default() -> Self {
        Self {
            endpoint: defaults::docker_endpoint(),
            network: None,
            poll_interval: defaults::docker_poll_interval(),
            reconnect_delay: defaults::docker_reconnect_delay(),
            allowed_domains: Vec::new(),
            inherit_forwarding: false,
        }
    }
}
