use std::time::Duration;

use infrarust_config::ProxyConfig;

fn load_proxy_fixture() -> ProxyConfig {
    let toml_str = include_str!("fixtures/infrarust.toml");
    toml::from_str(toml_str).expect("failed to parse infrarust.toml")
}

#[test]
fn test_parse_proxy_config() {
    let config = load_proxy_fixture();

    assert_eq!(config.bind, "0.0.0.0:25565".parse().unwrap());
    assert_eq!(config.max_connections, 10000);
    assert_eq!(config.connect_timeout, Duration::from_secs(5));
    assert!(!config.receive_proxy_protocol);
    assert_eq!(config.servers_dir.to_str().unwrap(), "./servers");
    assert_eq!(config.worker_threads, 0);
}

#[test]
fn test_parse_proxy_rate_limit() {
    let config = load_proxy_fixture();

    assert_eq!(config.rate_limit.max_connections, 3);
    assert_eq!(config.rate_limit.window, Duration::from_secs(10));
    assert_eq!(config.rate_limit.status_max, 30);
    assert_eq!(config.rate_limit.status_window, Duration::from_secs(10));
}

#[test]
fn test_parse_proxy_status_cache() {
    let config = load_proxy_fixture();

    assert_eq!(config.status_cache.ttl, Duration::from_secs(5));
    assert_eq!(config.status_cache.max_entries, 1000);
}

#[test]
fn test_parse_proxy_default_motd() {
    let config = load_proxy_fixture();

    let motd_config = config
        .default_motd
        .as_ref()
        .expect("default_motd should be set");
    let online = motd_config
        .online
        .as_ref()
        .expect("default_motd.online should be set");
    assert_eq!(online.text, "§cUnknown server");
    assert_eq!(online.version_name.as_deref(), Some("Infrarust"));
    assert_eq!(online.max_players, Some(0));
}

#[test]
fn test_parse_proxy_telemetry() {
    let config = load_proxy_fixture();

    let tc = config
        .telemetry
        .as_ref()
        .expect("telemetry should be present");
    assert!(!tc.enabled);
    assert_eq!(tc.endpoint.as_deref(), Some("http://localhost:4317"));
    assert_eq!(tc.protocol, "grpc");
    assert_eq!(tc.resource.service_name, "infrarust");
    assert_eq!(tc.resource.service_version, "2.0.0");
}

#[test]
fn test_proxy_defaults_minimal_config() {
    let config: ProxyConfig = toml::from_str("").expect("empty config should parse with defaults");

    assert_eq!(config.bind, "0.0.0.0:25565".parse().unwrap());
    assert_eq!(config.connect_timeout, Duration::from_secs(5));
    assert_eq!(config.max_connections, 0);
    assert!(!config.receive_proxy_protocol);
    assert_eq!(config.rate_limit.max_connections, 3);
    assert_eq!(config.status_cache.ttl, Duration::from_secs(5));
    assert!(config.default_motd.is_none());
}
