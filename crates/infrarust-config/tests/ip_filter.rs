use std::net::IpAddr;

use infrarust_config::IpFilterConfig;

#[test]
fn test_empty_filter_allows_all() {
    let filter = IpFilterConfig::default();
    let ip: IpAddr = "192.168.1.100".parse().unwrap();
    assert!(filter.is_allowed(&ip));
}

#[test]
fn test_whitelist_allows_match() {
    let filter = IpFilterConfig {
        whitelist: vec!["192.168.1.0/24".parse().unwrap()],
        blacklist: vec![],
    };
    let ip: IpAddr = "192.168.1.50".parse().unwrap();
    assert!(filter.is_allowed(&ip));
}

#[test]
fn test_whitelist_blocks_non_match() {
    let filter = IpFilterConfig {
        whitelist: vec!["192.168.1.0/24".parse().unwrap()],
        blacklist: vec![],
    };
    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    assert!(!filter.is_allowed(&ip));
}

#[test]
fn test_blacklist_blocks_match() {
    let filter = IpFilterConfig {
        whitelist: vec![],
        blacklist: vec!["10.0.99.0/24".parse().unwrap()],
    };
    let ip: IpAddr = "10.0.99.5".parse().unwrap();
    assert!(!filter.is_allowed(&ip));
}

#[test]
fn test_blacklist_allows_non_match() {
    let filter = IpFilterConfig {
        whitelist: vec![],
        blacklist: vec!["10.0.99.0/24".parse().unwrap()],
    };
    let ip: IpAddr = "10.0.1.1".parse().unwrap();
    assert!(filter.is_allowed(&ip));
}

#[test]
fn test_whitelist_priority_over_blacklist() {
    let filter = IpFilterConfig {
        whitelist: vec!["192.168.1.0/24".parse().unwrap()],
        blacklist: vec!["192.168.1.100/32".parse().unwrap()],
    };
    // When whitelist is non-empty, blacklist is ignored.
    // IP is in whitelist → allowed, even if it would be in blacklist.
    let ip: IpAddr = "192.168.1.100".parse().unwrap();
    assert!(filter.is_allowed(&ip));

    // IP not in whitelist → blocked (blacklist is not checked).
    let ip2: IpAddr = "10.0.0.1".parse().unwrap();
    assert!(!filter.is_allowed(&ip2));
}
