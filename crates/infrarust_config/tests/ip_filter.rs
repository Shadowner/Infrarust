#![allow(clippy::unwrap_used, clippy::expect_used)]
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
fn test_blacklist_applies_even_within_whitelist() {
    let filter = IpFilterConfig {
        whitelist: vec!["192.168.1.0/24".parse().unwrap()],
        blacklist: vec!["192.168.1.100/32".parse().unwrap()],
    };
    // Both lists apply: a blacklisted IP inside a whitelisted range is denied
    let blacklisted_in_whitelist: IpAddr = "192.168.1.100".parse().unwrap();
    assert!(!filter.is_allowed(&blacklisted_in_whitelist));

    // Whitelisted and not blacklisted → allowed.
    let whitelisted: IpAddr = "192.168.1.50".parse().unwrap();
    assert!(filter.is_allowed(&whitelisted));

    // Not in whitelist → blocked.
    let outside: IpAddr = "10.0.0.1".parse().unwrap();
    assert!(!filter.is_allowed(&outside));
}
