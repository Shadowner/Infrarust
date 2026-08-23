//! Parse tests for load-balancing configuration (§11 of the LB spec).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_config::{BalanceStrategy, ServerConfig, balance_warnings};

fn from_toml(toml: &str) -> ServerConfig {
    toml::from_str(toml).expect("failed to parse TOML")
}

#[test]
fn test_parse_addresses_simple_strings() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565", "10.0.0.2"]
    "#,
    );
    assert_eq!(config.addresses.len(), 2);
    assert!(config.addresses.iter().all(|a| a.weight == 1));
    assert_eq!(config.addresses[0].address.host, "10.0.0.1");
    assert_eq!(config.addresses[0].address.port, 25565);
    // No port → default Minecraft port
    assert_eq!(config.addresses[1].address.port, 25565);
}

#[test]
fn test_parse_addresses_weighted_table() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = [{ address = "10.0.0.1:25565", weight = 3 }]
    "#,
    );
    assert_eq!(config.addresses.len(), 1);
    assert_eq!(config.addresses[0].weight, 3);
    assert_eq!(config.addresses[0].address.host, "10.0.0.1");
}

#[test]
fn test_parse_addresses_mixed() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = [
            { address = "10.0.0.1:25565", weight = 3 },
            { address = "10.0.0.2:25565" },
            "10.0.0.3:25565",
        ]
    "#,
    );
    assert_eq!(config.addresses.len(), 3);
    assert_eq!(config.addresses[0].weight, 3);
    // Table without weight → implicit weight 1
    assert_eq!(config.addresses[1].weight, 1);
    assert_eq!(config.addresses[2].weight, 1);
    assert_eq!(config.addresses[2].address.host, "10.0.0.3");
}

#[test]
fn test_parse_balance_default_first_available() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565"]
    "#,
    );
    assert_eq!(config.balance, BalanceStrategy::FirstAvailable);
    assert_eq!(config.slow_start, None);
    assert!((config.slow_start_aggression - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_parse_balance_strategies() {
    for (raw, expected) in [
        ("first_available", BalanceStrategy::FirstAvailable),
        ("round_robin", BalanceStrategy::RoundRobin),
        ("least_conn", BalanceStrategy::LeastConn),
    ] {
        let config = from_toml(&format!(
            r#"
            domains = ["mc.example.com"]
            addresses = ["10.0.0.1:25565"]
            balance = "{raw}"
        "#
        ));
        assert_eq!(config.balance, expected);
    }
}

#[test]
fn test_parse_slow_start_duration() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565", "10.0.0.2:25565"]
        balance = "least_conn"
        slow_start = "45s"
        slow_start_aggression = 2.0
    "#,
    );
    assert_eq!(config.slow_start, Some(Duration::from_secs(45)));
    assert!((config.slow_start_aggression - 2.0).abs() < f64::EPSILON);
}

#[test]
fn test_balance_config_normalization() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565"]
        balance = "round_robin"
        slow_start = "30s"
    "#,
    );
    let balance = config.balance_config();
    assert_eq!(balance.strategy, BalanceStrategy::RoundRobin);
    assert_eq!(balance.slow_start, Some(Duration::from_secs(30)));
    assert!((balance.slow_start_aggression - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_address_list_strips_weights() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = [{ address = "10.0.0.1:25565", weight = 3 }, "10.0.0.2:25565"]
    "#,
    );
    let list = config.address_list();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].host, "10.0.0.1");
    assert_eq!(list[1].host, "10.0.0.2");
}

#[test]
fn test_warn_multi_address_first_available() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565", "10.0.0.2:25565"]
    "#,
    );
    let warnings = balance_warnings(&config);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("first_available"));
}

#[test]
fn test_warn_slow_start_with_first_available() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565"]
        slow_start = "45s"
    "#,
    );
    let warnings = balance_warnings(&config);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("slow_start"));
}

#[test]
fn test_warn_zero_weight() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        balance = "least_conn"
        addresses = [{ address = "10.0.0.1:25565", weight = 0 }, "10.0.0.2:25565"]
    "#,
    );
    let warnings = balance_warnings(&config);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("weight 0"));
}

#[test]
fn test_no_warnings_with_least_conn() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565", "10.0.0.2:25565"]
        balance = "least_conn"
        slow_start = "45s"
    "#,
    );
    assert!(balance_warnings(&config).is_empty());
}

#[test]
fn test_weighted_address_roundtrip_serialization() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = [{ address = "10.0.0.1:25565", weight = 3 }, "10.0.0.2:25565"]
        balance = "least_conn"
    "#,
    );
    let serialized = toml::to_string(&config).expect("serialize");
    let reparsed: ServerConfig = toml::from_str(&serialized).expect("reparse");
    assert_eq!(reparsed.addresses, config.addresses);
    assert_eq!(reparsed.balance, config.balance);
}

#[test]
fn test_invalid_aggression_rejected() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        proxy_mode = "client_only"
        addresses = ["10.0.0.1:25565"]
        slow_start_aggression = 0.0
    "#,
    );
    assert!(infrarust_config::validate_server_config(&config).is_err());
}

#[test]
fn test_weighted_address_unknown_field_rejected() {
    let result: Result<ServerConfig, _> = toml::from_str(
        r#"
        domains = ["mc.example.com"]
        addresses = [{ address = "10.0.0.1:25565", wieght = 3 }]
    "#,
    );
    assert!(
        result.is_err(),
        "typoed weight field must not be silently dropped"
    );
}
