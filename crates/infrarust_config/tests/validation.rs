#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_config::{ServerConfig, validate_server_config};

fn from_toml(toml: &str) -> ServerConfig {
    toml::from_str(toml).expect("failed to parse TOML")
}

#[test]
fn test_passthrough_without_domain_is_invalid() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "passthrough"
    "#,
    );
    assert!(config.domains.is_empty());
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_zerocopy_without_domain_is_invalid() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "zero_copy"
    "#,
    );
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_server_only_without_domain_is_invalid() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "server_only"
    "#,
    );
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_default_mode_without_domain_is_invalid() {
    // Default ProxyMode is Passthrough (forwarding)
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
    "#,
    );
    assert!(config.domains.is_empty());
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_client_only_without_domain_is_valid() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "client_only"
    "#,
    );
    assert!(config.domains.is_empty());
    assert!(validate_server_config(&config).is_ok());
}

#[test]
fn test_offline_without_domain_is_valid() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "offline"
    "#,
    );
    assert!(validate_server_config(&config).is_ok());
}

#[test]
fn test_full_without_domain_is_valid() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "full"
    "#,
    );
    assert!(validate_server_config(&config).is_ok());
}

#[test]
fn test_passthrough_with_domain_is_valid() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "passthrough"
    "#,
    );
    assert!(validate_server_config(&config).is_ok());
}

#[test]
fn test_toml_without_domains_field_deserializes_to_empty_vec() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "client_only"
    "#,
    );
    assert_eq!(config.domains, Vec::<String>::new());
}

#[test]
fn test_toml_with_domains_still_works() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com", "*.mc.example.com"]
        addresses = ["127.0.0.1:25565"]
    "#,
    );
    assert_eq!(config.domains.len(), 2);
    assert_eq!(config.domains[0], "mc.example.com");
    assert_eq!(config.domains[1], "*.mc.example.com");
}

#[test]
fn test_passthrough_with_network_is_invalid() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "passthrough"
        network = "main"
    "#,
    );
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_zerocopy_with_network_is_invalid() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "zero_copy"
        network = "main"
    "#,
    );
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_server_only_with_network_is_invalid() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "server_only"
        network = "main"
    "#,
    );
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_client_only_with_network_is_valid() {
    let config = from_toml(
        r#"
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "client_only"
        network = "main"
    "#,
    );
    assert!(validate_server_config(&config).is_ok());
}
