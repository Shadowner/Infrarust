#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_config::{
    ProxyConfig, ServerConfig, validate_proxy_config, validate_server_config,
    validate_server_configs,
};

fn from_toml(toml: &str) -> ServerConfig {
    toml::from_str(toml).expect("failed to parse TOML")
}

/// Parses a proxy config and points `servers_dir` at an existing directory
/// so that unrelated checks can be exercised.
fn proxy_from_toml(toml: &str, servers_dir: &std::path::Path) -> ProxyConfig {
    let mut config: ProxyConfig = toml::from_str(toml).expect("failed to parse TOML");
    config.servers_dir = servers_dir.to_path_buf();
    config
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

#[test]
fn test_id_with_invalid_charset_is_rejected() {
    let config = from_toml(
        r#"
        id = "My Server"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
    "#,
    );
    assert!(validate_server_config(&config).is_err());
}

#[test]
fn test_id_with_dots_is_valid() {
    // Filename-derived ids commonly contain dots (e.g. "1.20.4.toml").
    let config = from_toml(
        r#"
        id = "1.20.4"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
    "#,
    );
    assert!(validate_server_config(&config).is_ok());
}

#[test]
fn test_batch_validation_rejects_invalid_derived_id() {
    // The registry validates the batch after providers assign filename-derived
    // ids, so the charset check must also run there.
    let mut config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
    "#,
    );
    config.id = Some("My Server".to_string());
    assert!(validate_server_configs(std::slice::from_ref(&config)).is_err());
}

#[test]
fn test_proxy_minimal_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml("", dir.path());
    assert!(validate_proxy_config(&config).is_ok());
}

#[test]
fn test_proxy_zero_connect_timeout_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(r#"connect_timeout = "0s""#, dir.path());
    assert!(validate_proxy_config(&config).is_err());
}

#[test]
fn test_proxy_zero_rate_limit_window_is_invalid_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        r#"
        [rate_limit]
        enabled = true
        window = "0s"
    "#,
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_err());
}

#[test]
fn test_proxy_zero_rate_limit_window_is_ignored_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        r#"
        [rate_limit]
        enabled = false
        window = "0s"
    "#,
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_ok());
}

#[test]
fn test_proxy_zero_docker_poll_interval_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        r#"
        [docker]
        poll_interval = "0s"
    "#,
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_err());
}

#[test]
fn test_proxy_telemetry_protocol_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    for (protocol, ok) in [("grpc", true), ("http", true), ("udp", false)] {
        let config = proxy_from_toml(
            &format!("[telemetry]\nprotocol = \"{protocol}\""),
            dir.path(),
        );
        assert_eq!(
            validate_proxy_config(&config).is_ok(),
            ok,
            "telemetry.protocol = {protocol}"
        );
    }
}

#[test]
fn test_proxy_web_bind_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    for (bind, ok) in [
        ("127.0.0.1:8080", true),
        ("localhost:8080", true),
        ("[::1]:8080", true),
        ("nonsense", false),
        (":8080", false),
        ("127.0.0.1:notaport", false),
    ] {
        let config = proxy_from_toml(&format!("[web]\nbind = \"{bind}\""), dir.path());
        assert_eq!(
            validate_proxy_config(&config).is_ok(),
            ok,
            "web.bind = {bind}"
        );
    }
}

#[test]
fn test_proxy_web_bind_collision_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    // Default proxy bind is 0.0.0.0:25565, which covers every interface.
    let config = proxy_from_toml(
        r#"
        [web]
        bind = "127.0.0.1:25565"
    "#,
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_err());

    let config = proxy_from_toml(
        r#"
        bind = "127.0.0.1:25565"

        [web]
        bind = "192.168.1.5:25565"
    "#,
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_ok());
}
