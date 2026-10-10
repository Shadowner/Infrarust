use std::net::IpAddr;

#[must_use]
pub fn ip_in_range(addr: IpAddr, network: IpAddr, prefix: u8) -> bool {
    let (network, prefix) = canonical_network(network, prefix);
    match (network, addr.to_canonical()) {
        (IpAddr::V4(network), IpAddr::V4(addr)) if prefix <= 32 => {
            let mask = u32::MAX.checked_shl(u32::from(32 - prefix)).unwrap_or(0);
            u32::from(network) & mask == u32::from(addr) & mask
        }
        (IpAddr::V6(network), IpAddr::V6(addr)) if prefix <= 128 => {
            let mask = u128::MAX.checked_shl(u32::from(128 - prefix)).unwrap_or(0);
            u128::from(network) & mask == u128::from(addr) & mask
        }
        _ => false,
    }
}

fn canonical_network(network: IpAddr, prefix: u8) -> (IpAddr, u8) {
    match network {
        IpAddr::V6(v6) if prefix >= 96 => match v6.to_ipv4_mapped() {
            Some(v4) => (IpAddr::V4(v4), prefix - 96),
            None => (network, prefix),
        },
        other => (other, prefix),
    }
}

#[must_use]
pub fn parse_ip_range(range: &str) -> Option<(IpAddr, u8)> {
    let Some((network, prefix)) = range.split_once('/') else {
        let network = range.parse::<IpAddr>().ok()?;
        let width = if network.is_ipv4() { 32 } else { 128 };
        return Some((network, width));
    };
    Some((network.parse().ok()?, prefix.parse().ok()?))
}

#[must_use]
pub fn username_matches(banned: &str, attempt: &str) -> bool {
    banned.to_lowercase() == attempt.to_lowercase()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn in_range(range: &str, addr: &str) -> bool {
        parse_ip_range(range)
            .is_some_and(|(network, prefix)| ip_in_range(ip(addr), network, prefix))
    }

    #[test]
    fn an_ipv4_range_holds_the_addresses_inside_it_and_nothing_else() {
        assert!(in_range("10.20.0.0/16", "10.20.0.1"));
        assert!(in_range("10.20.0.0/16", "10.20.255.254"));
        assert!(!in_range("10.20.0.0/16", "10.21.0.1"));
        assert!(!in_range("10.20.0.0/16", "::1"));
        assert!(in_range("0.0.0.0/0", "192.0.2.5"));
        assert!(in_range("10.20.3.4/16", "10.20.9.9"));
    }

    #[test]
    fn an_ipv6_range_holds_the_addresses_inside_it_and_nothing_else() {
        assert!(in_range("2001:db8:abcd::/48", "2001:db8:abcd:12::1"));
        assert!(!in_range("2001:db8:abcd::/48", "2001:db8:abce::1"));
        assert!(!in_range("2001:db8:abcd::/48", "10.0.0.1"));
        assert!(!in_range("2001:db8::/32", "10.1.2.3"));
        assert!(!in_range("::/0", "10.1.2.3"));
        assert!(in_range("::/0", "2001:db8::1"));
    }

    #[test]
    fn a_v4_mapped_client_address_hits_the_ipv4_range() {
        assert!(in_range("10.20.0.0/16", "::ffff:10.20.3.4"));
        assert!(!in_range("10.21.0.0/16", "::ffff:10.20.3.4"));
        assert!(in_range("10.20.3.4", "::ffff:10.20.3.4"));
        assert!(!in_range("10.0.0.0/8", "2001:db8::1"));
    }

    #[test]
    fn a_v4_mapped_range_holds_plain_ipv4_clients() {
        assert!(in_range("::ffff:10.20.0.0/112", "10.20.9.9"));
        assert!(in_range("::ffff:10.20.0.0/112", "::ffff:10.20.9.9"));
        assert!(!in_range("::ffff:10.20.0.0/112", "10.21.0.1"));
        assert!(in_range("::ffff:10.0.0.0/104", "10.1.2.3"));
        assert!(!in_range("::ffff:10.0.0.0/104", "11.1.2.3"));
        assert!(in_range("::ffff:1.2.3.4", "1.2.3.4"));
    }

    #[test]
    fn a_bare_address_is_a_full_width_range() {
        assert_eq!(parse_ip_range("1.2.3.4"), Some((ip("1.2.3.4"), 32)));
        assert_eq!(
            parse_ip_range("2001:db8::1"),
            Some((ip("2001:db8::1"), 128))
        );
        assert_eq!(parse_ip_range("10.0.0.0/8"), Some((ip("10.0.0.0"), 8)));
        assert_eq!(parse_ip_range("nonsense"), None);
        assert_eq!(parse_ip_range("10.0.0.0/x"), None);
        assert_eq!(parse_ip_range("10.0.0.0/300"), None);
        assert!(!ip_in_range(ip("10.0.0.1"), ip("10.0.0.0"), 33));
        assert!(!ip_in_range(ip("::1"), ip("::"), 129));
    }

    #[test]
    fn usernames_match_case_insensitively_beyond_ascii() {
        assert!(username_matches("sTEVE", "Steve"));
        assert!(username_matches("strasse", "STRASSE"));
        assert!(username_matches("élodie", "ÉLODIE"));
        assert!(!username_matches("Alex", "Steve"));
    }
}
