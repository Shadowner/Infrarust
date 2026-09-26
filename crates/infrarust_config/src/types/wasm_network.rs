use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmNetworkConfig {
    #[serde(default)]
    pub allow: Vec<NetworkRule>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns: Option<bool>,

    #[serde(default = "defaults::wasm_network_http")]
    pub http: bool,
}

impl Default for WasmNetworkConfig {
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            dns: None,
            http: defaults::wasm_network_http(),
        }
    }
}

impl WasmNetworkConfig {
    #[must_use]
    pub fn dns_enabled(&self) -> bool {
        self.dns
            .unwrap_or_else(|| self.allow.iter().any(|rule| rule.hostname().is_some()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmMount {
    pub host: PathBuf,

    pub guest: String,

    #[serde(default = "defaults::wasm_mount_read_only")]
    pub read_only: bool,
}

impl WasmMount {
    pub fn guest_path(&self) -> Result<String, String> {
        let guest = &self.guest;
        if !guest.starts_with('/') {
            return Err(format!("guest path \"{guest}\" must be absolute"));
        }
        let mut parts = Vec::new();
        for part in guest.split('/').filter(|part| !part.is_empty()) {
            if part == "." || part == ".." {
                return Err(format!(
                    "guest path \"{guest}\" must not contain `.` or `..`"
                ));
            }
            parts.push(part);
        }
        if parts.is_empty() {
            return Err(format!(
                "guest path \"{guest}\" is the plugin data directory; mount somewhere below it"
            ));
        }
        Ok(format!("/{}", parts.join("/")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortRange {
    first: u16,
    last: u16,
}

impl PortRange {
    pub const ANY: Self = Self {
        first: 0,
        last: u16::MAX,
    };

    #[must_use]
    pub const fn single(port: u16) -> Self {
        Self {
            first: port,
            last: port,
        }
    }

    #[must_use]
    pub const fn contains(&self, port: u16) -> bool {
        self.first <= port && port <= self.last
    }

    #[must_use]
    pub const fn first(&self) -> u16 {
        self.first
    }

    #[must_use]
    pub const fn last(&self) -> u16 {
        self.last
    }
}

impl fmt::Display for PortRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::ANY {
            f.write_str("*")
        } else if self.first == self.last {
            write!(f, "{}", self.first)
        } else {
            write!(f, "{}-{}", self.first, self.last)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HostPattern {
    Ip(IpAddr),
    Net(IpNet),
    Name(String),
    Suffix(String),
}

impl fmt::Display for HostPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ip(IpAddr::V4(ip)) => write!(f, "{ip}"),
            Self::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]"),
            Self::Net(net @ IpNet::V4(_)) => write!(f, "{net}"),
            Self::Net(net @ IpNet::V6(_)) => write!(f, "[{net}]"),
            Self::Name(name) => f.write_str(name),
            Self::Suffix(suffix) => write!(f, "*.{suffix}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NetworkRule {
    host: HostPattern,
    ports: PortRange,
}

impl NetworkRule {
    #[must_use]
    pub fn host(&self) -> &HostPattern {
        &self.host
    }

    #[must_use]
    pub fn ports(&self) -> PortRange {
        self.ports
    }

    #[must_use]
    pub fn hostname(&self) -> Option<&str> {
        match &self.host {
            HostPattern::Name(name) => Some(name),
            _ => None,
        }
    }

    #[must_use]
    pub fn matches_ip(&self, ip: IpAddr, port: u16) -> bool {
        if !self.ports.contains(port) {
            return false;
        }
        let ip = ip.to_canonical();
        match &self.host {
            HostPattern::Ip(rule) => *rule == ip,
            HostPattern::Net(net) => net.contains(&ip),
            HostPattern::Name(_) | HostPattern::Suffix(_) => false,
        }
    }

    #[must_use]
    pub fn matches_name(&self, name: &str, port: u16) -> bool {
        if !self.ports.contains(port) {
            return false;
        }
        let name = normalize_name(name);
        match &self.host {
            HostPattern::Name(rule) => *rule == name,
            HostPattern::Suffix(suffix) => name
                .strip_suffix(suffix.as_str())
                .and_then(|label| label.strip_suffix('.'))
                .is_some_and(|label| !label.is_empty()),
            HostPattern::Ip(_) | HostPattern::Net(_) => false,
        }
    }

    #[must_use]
    pub fn permits_bind(&self, addr: SocketAddr) -> bool {
        matches!(self.host, HostPattern::Ip(rule) if rule == addr.ip().to_canonical())
            && self.ports.contains(addr.port())
    }
}

impl fmt::Display for NetworkRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.ports)
    }
}

impl From<NetworkRule> for String {
    fn from(rule: NetworkRule) -> Self {
        rule.to_string()
    }
}

impl TryFrom<String> for NetworkRule {
    type Error = NetworkRuleError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid network rule \"{rule}\": {reason}")]
pub struct NetworkRuleError {
    rule: String,
    reason: String,
}

impl NetworkRuleError {
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl FromStr for NetworkRule {
    type Err = NetworkRuleError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_rule(text).map_err(|reason| NetworkRuleError {
            rule: text.to_owned(),
            reason,
        })
    }
}

fn parse_rule(text: &str) -> Result<NetworkRule, String> {
    if text.is_empty() {
        return Err("expected host:port".to_owned());
    }
    if text.chars().any(char::is_whitespace) {
        return Err("must not contain whitespace".to_owned());
    }
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (inner, after) = rest
            .split_once(']')
            .ok_or_else(|| "missing the closing `]` of an IPv6 address".to_owned())?;
        let port = after
            .strip_prefix(':')
            .ok_or_else(|| "expected `:port` after the IPv6 address".to_owned())?;
        (parse_ipv6_host(inner)?, port)
    } else {
        let (host, port) = text
            .rsplit_once(':')
            .ok_or_else(|| "missing `:port` (write `:*` for any port)".to_owned())?;
        if host.contains(':') {
            return Err("IPv6 addresses must be written in brackets, e.g. [::1]:443".to_owned());
        }
        (parse_host(host)?, port)
    };
    Ok(NetworkRule {
        host,
        ports: parse_ports(port)?,
    })
}

fn parse_ipv6_host(inner: &str) -> Result<HostPattern, String> {
    let host = if inner.contains('/') {
        match inner.parse::<IpNet>() {
            Ok(net @ IpNet::V6(_)) => {
                check_prefix(net)?;
                HostPattern::Net(net)
            }
            Ok(IpNet::V4(_)) => {
                return Err("brackets are only for IPv6; write IPv4 ranges as a.b.c.d/n".to_owned());
            }
            Err(_) => return Err(format!("\"{inner}\" is not a valid IPv6 range")),
        }
    } else {
        let ip: Ipv6Addr = inner
            .parse()
            .map_err(|_| format!("\"{inner}\" is not a valid IPv6 address"))?;
        HostPattern::Ip(IpAddr::V6(ip))
    };
    let mapped = match &host {
        HostPattern::Ip(IpAddr::V6(ip)) => ip.to_ipv4_mapped().is_some(),
        HostPattern::Net(IpNet::V6(net)) => {
            net.prefix_len() >= 96 && net.network().to_ipv4_mapped().is_some()
        }
        _ => false,
    };
    if mapped {
        return Err(
            "IPv4-mapped IPv6 addresses are matched as IPv4; write the a.b.c.d form".to_owned(),
        );
    }
    Ok(host)
}

fn parse_host(host: &str) -> Result<HostPattern, String> {
    if host.is_empty() {
        return Err("missing host".to_owned());
    }
    if host == "*" {
        return Err(
            "a bare `*` host is not allowed; write 0.0.0.0/0:* or [::/0]:* to allow every address"
                .to_owned(),
        );
    }
    if host.contains('/') {
        return match host.parse::<IpNet>() {
            Ok(net @ IpNet::V4(_)) => {
                check_prefix(net)?;
                Ok(HostPattern::Net(net))
            }
            _ => Err(format!("\"{host}\" is not a valid IPv4 range")),
        };
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(HostPattern::Ip(IpAddr::V4(ip)));
    }
    if let Some(suffix) = host.strip_prefix("*.") {
        if suffix.contains('*') {
            return Err(wildcard_error());
        }
        return Ok(HostPattern::Suffix(parse_hostname(suffix)?));
    }
    if host.contains('*') {
        return Err(wildcard_error());
    }
    Ok(HostPattern::Name(parse_hostname(host)?))
}

fn wildcard_error() -> String {
    "a wildcard is only allowed as one leading `*.` label, e.g. *.example.org".to_owned()
}

fn check_prefix(net: IpNet) -> Result<(), String> {
    if net.trunc() == net {
        Ok(())
    } else {
        Err(format!(
            "the address has bits set past the /{} prefix; did you mean {}?",
            net.prefix_len(),
            HostPattern::Net(net.trunc())
        ))
    }
}

fn parse_hostname(name: &str) -> Result<String, String> {
    let name = normalize_name(name);
    if name.is_empty() {
        return Err("missing host".to_owned());
    }
    if name.len() > 253 {
        return Err("a hostname is at most 253 characters".to_owned());
    }
    for label in name.split('.') {
        let valid = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(format!("\"{name}\" is not a valid hostname"));
        }
    }
    if name
        .rsplit('.')
        .next()
        .is_some_and(|last| last.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(format!(
            "\"{name}\" is neither a valid IPv4 address nor a hostname"
        ));
    }
    Ok(name)
}

fn normalize_name(name: &str) -> String {
    name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase()
}

fn parse_ports(port: &str) -> Result<PortRange, String> {
    let number = |text: &str| {
        text.parse::<u16>().map_err(|_| {
            format!("invalid port \"{port}\": expected a number from 0 to 65535, a range a-b, or *")
        })
    };
    if port == "*" {
        return Ok(PortRange::ANY);
    }
    if port.is_empty() {
        return Err("missing port after `:` (write `:*` for any port)".to_owned());
    }
    match port.split_once('-') {
        Some((first, last)) => {
            let (first, last) = (number(first)?, number(last)?);
            if first > last {
                return Err(format!("port range \"{port}\" starts after it ends"));
            }
            Ok(PortRange { first, last })
        }
        None => number(port).map(PortRange::single),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> NetworkRule {
        text.parse()
            .unwrap_or_else(|e| panic!("{text} should parse: {e}"))
    }

    fn reason(text: &str) -> String {
        let error = text
            .parse::<NetworkRule>()
            .expect_err("the rule should be rejected");
        let message = error.to_string();
        assert!(
            message.starts_with(&format!("invalid network rule \"{text}\": ")),
            "{message}"
        );
        error.reason().to_owned()
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn every_host_form_parses_and_prints_back() {
        for text in [
            "127.0.0.1:5432",
            "10.0.0.0/8:3306",
            "[::1]:*",
            "[fd00::/8]:443",
            "db.internal:5432",
            "api.example.com:443",
            "*.example.org:443",
            "10.1.2.3:8000-8100",
            "0.0.0.0/0:*",
            "[::/0]:*",
        ] {
            let parsed = rule(text);
            assert_eq!(parsed.to_string(), text);
            assert_eq!(rule(&parsed.to_string()), parsed);
        }
    }

    #[test]
    fn names_are_lowercased_and_lose_a_trailing_dot() {
        assert_eq!(rule("DB.Internal.:5432").to_string(), "db.internal:5432");
        assert_eq!(rule("*.Example.ORG:443").to_string(), "*.example.org:443");
    }

    #[test]
    fn port_forms() {
        assert_eq!(rule("1.2.3.4:*").ports(), PortRange::ANY);
        assert_eq!(rule("1.2.3.4:0-65535").ports(), PortRange::ANY);
        assert_eq!(rule("1.2.3.4:80").ports(), PortRange::single(80));
        let range = rule("1.2.3.4:8000-8100").ports();
        assert_eq!((range.first(), range.last()), (8000, 8100));
        assert!(range.contains(8000) && range.contains(8100));
        assert!(!range.contains(7999) && !range.contains(8101));
    }

    #[test]
    fn malformed_rules_are_rejected_with_a_reason() {
        assert_eq!(
            reason("*:443"),
            "a bare `*` host is not allowed; write 0.0.0.0/0:* or [::/0]:* to allow every address"
        );
        assert_eq!(reason(""), "expected host:port");
        assert_eq!(
            reason("1.2.3.4"),
            "missing `:port` (write `:*` for any port)"
        );
        assert_eq!(reason(":80"), "missing host");
        assert_eq!(
            reason("1.2.3.4:"),
            "missing port after `:` (write `:*` for any port)"
        );
        assert_eq!(
            reason("1.2.3.4:http"),
            "invalid port \"http\": expected a number from 0 to 65535, a range a-b, or *"
        );
        assert_eq!(
            reason("1.2.3.4:70000"),
            "invalid port \"70000\": expected a number from 0 to 65535, a range a-b, or *"
        );
        assert_eq!(
            reason("1.2.3.4:90-80"),
            "port range \"90-80\" starts after it ends"
        );
        assert_eq!(
            reason("::1:80"),
            "IPv6 addresses must be written in brackets, e.g. [::1]:443"
        );
        assert_eq!(
            reason("[::1:80"),
            "missing the closing `]` of an IPv6 address"
        );
        assert_eq!(reason("[::1]80"), "expected `:port` after the IPv6 address");
        assert_eq!(
            reason("[1.2.3.4]:80"),
            "\"1.2.3.4\" is not a valid IPv6 address"
        );
        assert_eq!(
            reason("[10.0.0.0/8]:80"),
            "brackets are only for IPv6; write IPv4 ranges as a.b.c.d/n"
        );
        assert_eq!(
            reason("[fd00::/200]:80"),
            "\"fd00::/200\" is not a valid IPv6 range"
        );
        assert_eq!(
            reason("10.0.0.0/33:80"),
            "\"10.0.0.0/33\" is not a valid IPv4 range"
        );
        assert_eq!(
            reason("10.0.0.1/8:80"),
            "the address has bits set past the /8 prefix; did you mean 10.0.0.0/8?"
        );
        assert_eq!(
            reason("[fd00::1/8]:80"),
            "the address has bits set past the /8 prefix; did you mean [fd00::/8]?"
        );
        assert_eq!(
            reason("[::ffff:127.0.0.1]:80"),
            "IPv4-mapped IPv6 addresses are matched as IPv4; write the a.b.c.d form"
        );
        assert_eq!(
            reason("[::ffff:10.0.0.0/104]:80"),
            "IPv4-mapped IPv6 addresses are matched as IPv4; write the a.b.c.d form"
        );
        assert_eq!(
            reason("a*.example.org:443"),
            "a wildcard is only allowed as one leading `*.` label, e.g. *.example.org"
        );
        assert_eq!(
            reason("*.*.example.org:443"),
            "a wildcard is only allowed as one leading `*.` label, e.g. *.example.org"
        );
        assert_eq!(reason("*.:443"), "missing host");
        assert_eq!(
            reason("bad_-.host-:80"),
            "\"bad_-.host-\" is not a valid hostname"
        );
        assert_eq!(reason("a..b:80"), "\"a..b\" is not a valid hostname");
        assert_eq!(
            reason("999.1.1.1:80"),
            "\"999.1.1.1\" is neither a valid IPv4 address nor a hostname"
        );
        assert_eq!(reason("db internal:80"), "must not contain whitespace");
        let long = format!("{}.com:80", "a".repeat(250));
        assert_eq!(reason(&long), "a hostname is at most 253 characters");
    }

    #[test]
    fn ipv4_rules_match_addresses_and_ports() {
        let exact = rule("127.0.0.1:5432");
        assert!(exact.matches_ip(ip("127.0.0.1"), 5432));
        assert!(!exact.matches_ip(ip("127.0.0.1"), 5433));
        assert!(!exact.matches_ip(ip("127.0.0.2"), 5432));
        assert!(!exact.matches_name("127.0.0.1", 5432));
    }

    #[test]
    fn cidr_edges_are_inclusive_and_nothing_past_them_matches() {
        let net = rule("10.0.0.0/8:*");
        assert!(net.matches_ip(ip("10.0.0.0"), 1));
        assert!(net.matches_ip(ip("10.255.255.255"), 65535));
        assert!(!net.matches_ip(ip("9.255.255.255"), 1));
        assert!(!net.matches_ip(ip("11.0.0.0"), 1));
        assert!(!net.matches_ip(ip("::a00:1"), 1));

        let slash32 = rule("192.168.1.7/32:22");
        assert!(slash32.matches_ip(ip("192.168.1.7"), 22));
        assert!(!slash32.matches_ip(ip("192.168.1.8"), 22));

        let everything = rule("0.0.0.0/0:*");
        assert!(everything.matches_ip(ip("203.0.113.9"), 443));
        assert!(!everything.matches_ip(ip("2001:db8::1"), 443));
    }

    #[test]
    fn ipv6_rules_match_ipv6_only() {
        let loopback = rule("[::1]:*");
        assert!(loopback.matches_ip(ip("::1"), 1234));
        assert!(!loopback.matches_ip(ip("127.0.0.1"), 1234));

        let net = rule("[fd00::/8]:443");
        assert!(net.matches_ip(ip("fd00::"), 443));
        assert!(net.matches_ip(ip("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), 443));
        assert!(!net.matches_ip(ip("fe00::"), 443));
        assert!(!net.matches_ip(ip("fd00::1"), 80));

        let every_v6 = rule("[::/0]:*");
        assert!(every_v6.matches_ip(ip("2001:db8::1"), 1));
        assert!(
            !every_v6.matches_ip(ip("127.0.0.1"), 1),
            "[::/0] does not reach IPv4 through the mapped range"
        );
        assert!(!every_v6.matches_ip(ip("::ffff:127.0.0.1"), 1));
    }

    #[test]
    fn ipv4_mapped_ipv6_addresses_match_as_ipv4() {
        let exact = rule("127.0.0.1:80");
        assert!(exact.matches_ip(ip("::ffff:127.0.0.1"), 80));
        assert!(!exact.matches_ip(ip("::ffff:127.0.0.2"), 80));
        assert!(rule("10.0.0.0/8:*").matches_ip(ip("::ffff:10.9.8.7"), 1));
    }

    #[test]
    fn hostname_rules_match_the_exact_name_only() {
        let db = rule("db.internal:5432");
        assert_eq!(db.hostname(), Some("db.internal"));
        assert!(db.matches_name("db.internal", 5432));
        assert!(db.matches_name("DB.INTERNAL.", 5432));
        assert!(!db.matches_name("db.internal", 5433));
        assert!(!db.matches_name("x.db.internal", 5432));
        assert!(!db.matches_name("db.internal.evil", 5432));
        assert!(!db.matches_ip(ip("127.0.0.1"), 5432));
    }

    #[test]
    fn a_wildcard_matches_subdomains_but_not_the_suffix_or_a_lookalike() {
        let wild = rule("*.example.org:443");
        assert_eq!(wild.hostname(), None);
        assert!(wild.matches_name("api.example.org", 443));
        assert!(wild.matches_name("a.b.example.org", 443));
        assert!(!wild.matches_name("example.org", 443));
        assert!(!wild.matches_name(".example.org", 443));
        assert!(!wild.matches_name("evilexample.org", 443));
        assert!(!wild.matches_name("example.org.evil", 443));
        assert!(!wild.matches_name("api.example.org", 80));
    }

    #[test]
    fn binds_need_an_exact_address_rule() {
        let addr = |text: &str| text.parse::<SocketAddr>().unwrap();
        assert!(rule("127.0.0.1:9000-9100").permits_bind(addr("127.0.0.1:9050")));
        assert!(!rule("127.0.0.1:9000-9100").permits_bind(addr("127.0.0.1:9200")));
        assert!(rule("0.0.0.0:25600").permits_bind(addr("0.0.0.0:25600")));
        assert!(!rule("0.0.0.0/0:*").permits_bind(addr("0.0.0.0:25600")));
        assert!(!rule("127.0.0.0/8:*").permits_bind(addr("127.0.0.1:9000")));
        assert!(!rule("localhost:*").permits_bind(addr("127.0.0.1:9000")));
        assert!(rule("[::1]:*").permits_bind(addr("[::1]:7000")));
    }

    #[test]
    fn dns_defaults_to_on_only_with_an_exact_hostname_rule() {
        let config = |allow: &[&str], dns: Option<bool>| WasmNetworkConfig {
            allow: allow.iter().map(|text| rule(text)).collect(),
            dns,
            http: true,
        };
        assert!(!config(&[], None).dns_enabled());
        assert!(!config(&["127.0.0.1:80", "*.example.org:443"], None).dns_enabled());
        assert!(config(&["db.internal:5432"], None).dns_enabled());
        assert!(!config(&["db.internal:5432"], Some(false)).dns_enabled());
        assert!(config(&["127.0.0.1:80"], Some(true)).dns_enabled());
    }

    #[test]
    fn guest_paths_are_normalized_and_checked() {
        let mount = |guest: &str| WasmMount {
            host: PathBuf::from("/srv"),
            guest: guest.to_owned(),
            read_only: true,
        };
        assert_eq!(mount("/shared").guest_path().unwrap(), "/shared");
        assert_eq!(mount("//a//b/").guest_path().unwrap(), "/a/b");
        assert_eq!(
            mount("shared").guest_path().unwrap_err(),
            "guest path \"shared\" must be absolute"
        );
        assert_eq!(
            mount("/").guest_path().unwrap_err(),
            "guest path \"/\" is the plugin data directory; mount somewhere below it"
        );
        assert_eq!(
            mount("/a/../b").guest_path().unwrap_err(),
            "guest path \"/a/../b\" must not contain `.` or `..`"
        );
        assert_eq!(
            mount("/a/./b").guest_path().unwrap_err(),
            "guest path \"/a/./b\" must not contain `.` or `..`"
        );
    }
}
