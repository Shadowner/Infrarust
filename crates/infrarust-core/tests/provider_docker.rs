#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "docker")]

use std::collections::HashMap;

use infrarust_config::{DockerProviderConfig, ForwardingMode, validate_server_forwarding};
use infrarust_core::provider::docker::{
    DomainsNotAllowed, container_server_config, labels_to_server_config, resolve_container_address,
};

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

    assert_eq!(config.addresses[0].address.port, 25566);
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
fn test_labels_to_config_motd_preserved() {
    let labels = make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.motd.text", "Welcome!"),
    ]);
    let config = labels_to_server_config("mc-test", &labels, "172.17.0.2:25565");

    assert_eq!(config.id.as_deref(), Some("mc-test"));
    assert_eq!(config.motd.online.unwrap().text, "Welcome!");
}

#[test]
fn test_labels_to_config_special_chars_round_trip() {
    let motd = "He said \"hi\"\nC:\\path [section]";
    let labels = make_labels(&[("infrarust.enable", "true"), ("infrarust.motd.text", motd)]);
    let config = labels_to_server_config("mc-test", &labels, "172.17.0.2:25565");

    assert_eq!(config.motd.online.unwrap().text, motd);
}

#[test]
fn test_labels_to_config_proxy_mode() {
    let labels = make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.proxy_mode", "client_only"),
    ]);
    let config = labels_to_server_config("mc-test", &labels, "172.17.0.2:25565");

    assert_eq!(config.proxy_mode, infrarust_config::ProxyMode::ClientOnly);
}

#[test]
fn test_address_resolution_never_falls_back_to_the_container_name() {
    use bollard::models::ContainerInspectResponse;

    let info = ContainerInspectResponse {
        name: Some("/mc-survival".to_string()),
        ..Default::default()
    };

    assert_eq!(resolve_container_address(&info, None, 25565), None);
}

#[test]
fn test_address_resolution_uses_a_published_port_without_a_network_ip() {
    use bollard::models::{ContainerInspectResponse, HostConfig, PortBinding};

    let binding = PortBinding {
        host_ip: Some("0.0.0.0".to_string()),
        host_port: Some("30000".to_string()),
    };
    let info = ContainerInspectResponse {
        name: Some("/mc-survival".to_string()),
        host_config: Some(HostConfig {
            port_bindings: Some(HashMap::from([(
                "25565/tcp".to_string(),
                Some(vec![binding]),
            )])),
            ..Default::default()
        }),
        ..Default::default()
    };

    assert_eq!(
        resolve_container_address(&info, None, 25565).as_deref(),
        Some("127.0.0.1:30000")
    );
}

#[test]
fn test_address_resolution_picks_first_network_by_name() {
    use bollard::models::{ContainerInspectResponse, EndpointSettings, NetworkSettings};

    let endpoint = |ip: &str| EndpointSettings {
        ip_address: Some(ip.to_string()),
        ..Default::default()
    };
    let networks = HashMap::from([
        ("zeta".to_string(), endpoint("10.0.0.3")),
        ("alpha".to_string(), endpoint("")),
        ("beta".to_string(), endpoint("10.0.0.2")),
        ("gamma".to_string(), endpoint("10.0.0.1")),
    ]);
    let info = ContainerInspectResponse {
        network_settings: Some(NetworkSettings {
            networks: Some(networks),
            ..Default::default()
        }),
        ..Default::default()
    };

    let addr = resolve_container_address(&info, None, 25565);
    assert_eq!(addr.as_deref(), Some("10.0.0.2:25565"));
}

fn policy(allowed_domains: &[&str]) -> DockerProviderConfig {
    DockerProviderConfig {
        allowed_domains: allowed_domains.iter().map(ToString::to_string).collect(),
        ..Default::default()
    }
}

fn claiming(domains: &str) -> HashMap<String, String> {
    make_labels(&[("infrarust.enable", "true"), ("infrarust.domains", domains)])
}

const ADDRESS: &str = "172.17.0.2:25565";

#[test]
fn test_policy_refuses_label_domains_by_default() {
    let refusal = container_server_config(
        &DockerProviderConfig::default(),
        "mc-evil",
        &claiming("play.mc.com"),
        ADDRESS,
    )
    .unwrap_err();

    assert_eq!(
        refusal,
        DomainsNotAllowed {
            allowing_line: r#"allowed_domains = ["play.mc.com"]"#.to_string()
        }
    );
}

#[test]
fn test_policy_allowing_line_keeps_the_existing_entries() {
    let refusal = container_server_config(
        &policy(&["*.lan.mc.com"]),
        "mc-test",
        &claiming("a.lan.mc.com, Play.MC.com"),
        ADDRESS,
    )
    .unwrap_err();

    assert_eq!(
        refusal.allowing_line,
        r#"allowed_domains = ["*.lan.mc.com", "play.mc.com"]"#
    );
}

#[test]
fn test_policy_routes_the_automatic_domain_without_an_allowlist() {
    let labels = make_labels(&[("infrarust.enable", "true")]);
    let config = container_server_config(
        &DockerProviderConfig::default(),
        "mc-test",
        &labels,
        ADDRESS,
    )
    .unwrap();

    assert_eq!(config.domains, vec!["mc-test.docker.local"]);
}

#[test]
fn test_policy_does_not_extend_the_automatic_domain_to_other_containers() {
    let refusal = container_server_config(
        &DockerProviderConfig::default(),
        "mc-evil",
        &claiming("mc-other.docker.local"),
        ADDRESS,
    );

    assert!(refusal.is_err());
}

#[test]
fn test_policy_allows_listed_domains_and_patterns() {
    let config = container_server_config(
        &policy(&["Exact.mc.com", "*.lan.mc.com"]),
        "mc-test",
        &claiming("exact.mc.com, a.lan.mc.com"),
        ADDRESS,
    )
    .unwrap();

    assert_eq!(config.domains, vec!["exact.mc.com", "a.lan.mc.com"]);
}

#[test]
fn test_policy_refuses_one_outside_domain_among_allowed_ones() {
    let refusal = container_server_config(
        &policy(&["*.lan.mc.com"]),
        "mc-test",
        &claiming("a.lan.mc.com, play.mc.com"),
        ADDRESS,
    );

    assert!(refusal.is_err());
}

#[test]
fn test_policy_refuses_a_label_pattern_an_entry_only_matches_as_text() {
    for (entry, label) in [
        ("?", "*"),
        ("*.mc.com", "*.mc.com*"),
        ("?.mc.com", "*.mc.com"),
    ] {
        let refusal =
            container_server_config(&policy(&[entry]), "mc-test", &claiming(label), ADDRESS);
        assert!(refusal.is_err(), "{entry} must not admit {label}");
    }

    let same_pattern = container_server_config(
        &policy(&["*.lan.mc.com"]),
        "mc-test",
        &claiming("*.lan.mc.com"),
        ADDRESS,
    );
    assert!(same_pattern.is_ok());
}

#[test]
fn test_policy_keeps_containers_out_of_proxy_wide_forwarding() {
    let labels = make_labels(&[("infrarust.enable", "true")]);
    let config = container_server_config(
        &DockerProviderConfig::default(),
        "mc-test",
        &labels,
        ADDRESS,
    )
    .unwrap();

    assert_eq!(config.forwarding_mode, Some(ForwardingMode::None));
    for global in [
        ForwardingMode::None,
        ForwardingMode::BungeeCord,
        ForwardingMode::BungeeGuard,
        ForwardingMode::Velocity,
    ] {
        assert!(
            validate_server_forwarding(&config, global.clone()).is_ok(),
            "{global:?}"
        );
    }
}

#[test]
fn test_policy_inherit_forwarding_leaves_the_mode_unset() {
    let labels = make_labels(&[("infrarust.enable", "true")]);
    let policy = DockerProviderConfig {
        inherit_forwarding: true,
        ..Default::default()
    };
    let config = container_server_config(&policy, "mc-test", &labels, ADDRESS).unwrap();

    assert_eq!(config.forwarding_mode, None);
}

#[test]
fn test_policy_settings_default_to_closed() {
    let parsed: DockerProviderConfig = toml::from_str("").unwrap();

    assert!(parsed.allowed_domains.is_empty());
    assert!(!parsed.inherit_forwarding);
}
