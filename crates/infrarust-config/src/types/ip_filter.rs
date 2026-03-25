//! IP filtering configuration.

use ipnet::IpNet;
use serde::Deserialize;

/// IP filtering by CIDR.
///
/// If `whitelist` is non-empty, only IPs in the whitelist are allowed.
/// If `blacklist` is non-empty, IPs in the blacklist are rejected.
/// The whitelist is evaluated first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
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
