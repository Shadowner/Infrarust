#![cfg(feature = "docker")]

use std::collections::HashMap;

use infrarust_core::provider::docker::{labels_to_server_config, resolve_container_address};

fn make_labels(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn test_labels_to_config_minimal() {
    let labels = make_labels(&[("infrarust.enable", "true")]);
    let config = labels_to_server_config("mc-test", &labels, "172.17.0.2:25565");

    assert_eq!(config.domains, vec!["mc-test.docker.local"]);
}

#[test]
fn test_labels_to_config_auto_domain() {
    let labels = make_labels(&[("infrarust.enable", "true")]);
    let config = labels_to_server_config("my-server", &labels, "172.17.0.5:25565");

    assert_eq!(config.domains, vec!["my-server.docker.local"]);
}

#[test]
fn test_labels_to_config_multi_domains() {
    let labels = make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.domains", "a.com, b.com, c.com"),
    ]);
    let config = labels_to_server_config("mc-test", &labels, "172.17.0.2:25565");

    assert_eq!(config.domains, vec!["a.com", "b.com", "c.com"]);
}

#[test]
fn test_labels_to_config_custom_port() {
    let labels = make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.domains", "test.mc.local"),
    ]);
    let config = labels_to_server_config("mc-test", &labels, "172.17.0.2:25566");

    assert_eq!(config.addresses[0].port, 25566);
}

#[test]
fn test_labels_to_config_proxy_protocol() {
    let labels = make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.send_proxy_protocol", "true"),
    ]);
    let config = labels_to_server_config("mc-test", &labels, "172.17.0.2:25565");

    assert!(config.send_proxy_protocol);
}

#[test]
fn test_address_resolution_hostname_fallback() {
    use bollard::models::ContainerInspectResponse;

    // Empty container info → falls back to container name
    let info = ContainerInspectResponse {
        name: Some("/mc-survival".to_string()),
        ..Default::default()
    };

    let addr = resolve_container_address(&info, None, 25565);
    assert_eq!(addr, "mc-survival:25565");
}
