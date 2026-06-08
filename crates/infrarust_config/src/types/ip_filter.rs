//! IP filtering configuration.

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

/// IP filtering by CIDR.
///
/// An IP is allowed when it is **not** in the `blacklist` **and** (the
/// `whitelist` is empty **or** the IP is in the `whitelist`). The blacklist
/// always applies, so a blacklisted IP inside a whitelisted range is still
/// rejected — both lists can be used together.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
        if self.blacklist.iter().any(|net| net.contains(ip)) {
            return false;
        }
        if self.whitelist.is_empty() {
            return true;
        }
        self.whitelist.iter().any(|net| net.contains(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn net(s: &str) -> IpNet {
        s.parse().expect("valid CIDR")
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("valid IP")
    }

    #[test]
    fn empty_filter_allows_all() {
        let f = IpFilterConfig::default();
        assert!(f.is_allowed(&ip("1.2.3.4")));
    }

    #[test]
    fn whitelist_only_restricts() {
        let f = IpFilterConfig {
            whitelist: vec![net("10.0.0.0/8")],
            blacklist: vec![],
        };
        assert!(f.is_allowed(&ip("10.1.2.3")));
        assert!(!f.is_allowed(&ip("192.168.0.1")));
    }

    #[test]
    fn blacklist_only_denies_listed() {
        let f = IpFilterConfig {
            whitelist: vec![],
            blacklist: vec![net("192.168.0.0/16")],
        };
        assert!(!f.is_allowed(&ip("192.168.5.5")));
        assert!(f.is_allowed(&ip("10.0.0.1")));
    }

    #[test]
    fn blacklist_wins_over_whitelist() {
        let f = IpFilterConfig {
            whitelist: vec![net("10.0.0.0/8")],
            blacklist: vec![net("10.6.6.0/24")],
        };
        assert!(
            f.is_allowed(&ip("10.1.1.1")),
            "whitelisted, not blacklisted"
        );
        assert!(
            !f.is_allowed(&ip("10.6.6.6")),
            "blacklisted IP inside a whitelisted range must be denied"
        );
        assert!(!f.is_allowed(&ip("8.8.8.8")), "outside the whitelist");
    }
}
