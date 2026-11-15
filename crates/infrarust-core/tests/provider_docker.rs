#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "docker")]

use std::collections::HashMap;

use infrarust_config::{DockerProviderConfig, ForwardingMode, validate_server_forwarding};
use infrarust_core::provider::ConfigProvider;
use infrarust_core::provider::docker::{
    DockerProvider, DomainsNotAllowed, container_server_config, labels_to_server_config,
    resolve_container_address, withheld_forwarding_warning,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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

fn port_map(host_ip: &str, host_port: &str) -> bollard::models::PortMap {
    let binding = bollard::models::PortBinding {
        host_ip: Some(host_ip.to_string()),
        host_port: Some(host_port.to_string()),
    };
    HashMap::from([("25565/tcp".to_string(), Some(vec![binding]))])
}

#[test]
fn test_address_resolution_uses_a_published_port_without_a_network_ip() {
    use bollard::models::{ContainerInspectResponse, NetworkSettings};

    let info = ContainerInspectResponse {
        name: Some("/mc-survival".to_string()),
        network_settings: Some(NetworkSettings {
            ports: Some(port_map("0.0.0.0", "30000")),
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
fn test_address_resolution_ignores_requested_bindings_the_daemon_did_not_publish() {
    use bollard::models::{
        ContainerInspectResponse, EndpointSettings, HostConfig, NetworkSettings,
    };

    for (host_ip, host_port) in [("0.0.0.0", "25566"), ("10.0.0.5", "22")] {
        let info = ContainerInspectResponse {
            host_config: Some(HostConfig {
                port_bindings: Some(port_map(host_ip, host_port)),
                ..Default::default()
            }),
            network_settings: Some(NetworkSettings {
                networks: Some(HashMap::from([(
                    "none".to_string(),
                    EndpointSettings {
                        ip_address: Some(String::new()),
                        ..Default::default()
                    },
                )])),
                ports: Some(HashMap::new()),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            resolve_container_address(&info, None, 25565),
            None,
            "{host_ip}:{host_port}"
        );
    }
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

fn inheriting_policy() -> DockerProviderConfig {
    DockerProviderConfig {
        inherit_forwarding: true,
        ..Default::default()
    }
}

fn labels_in_mode(proxy_mode: &str) -> HashMap<String, String> {
    make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.proxy_mode", proxy_mode),
    ])
}

#[test]
fn test_policy_inherit_forwarding_leaves_the_mode_unset_on_intercepted_modes() {
    for proxy_mode in ["client_only", "offline"] {
        let config = container_server_config(
            &inheriting_policy(),
            "mc-test",
            &labels_in_mode(proxy_mode),
            ADDRESS,
        )
        .unwrap();

        assert_eq!(config.forwarding_mode, None, "{proxy_mode}");
    }
}

#[test]
fn test_policy_inherit_forwarding_keeps_forwarding_modes_routable() {
    let unlabelled = make_labels(&[("infrarust.enable", "true")]);
    for labels in [
        unlabelled,
        labels_in_mode("passthrough"),
        labels_in_mode("zero_copy"),
        labels_in_mode("server_only"),
    ] {
        let config =
            container_server_config(&inheriting_policy(), "mc-test", &labels, ADDRESS).unwrap();

        assert_eq!(
            config.forwarding_mode,
            Some(ForwardingMode::None),
            "{labels:?}"
        );
        for global in [
            ForwardingMode::None,
            ForwardingMode::BungeeCord,
            ForwardingMode::BungeeGuard,
            ForwardingMode::Velocity,
        ] {
            assert!(
                validate_server_forwarding(&config, global.clone()).is_ok(),
                "{labels:?} under {global:?}"
            );
        }
    }
}

#[test]
fn test_inherit_forwarding_warns_about_containers_it_cannot_reach() {
    let policy = inheriting_policy();
    let config =
        container_server_config(&policy, "mc-test", &labels_in_mode("passthrough"), ADDRESS)
            .unwrap();

    for proxy_wide in [
        ForwardingMode::BungeeCord,
        ForwardingMode::BungeeGuard,
        ForwardingMode::Velocity,
    ] {
        let warning =
            withheld_forwarding_warning(&policy, &proxy_wide, "mc-test", &config).unwrap();
        assert!(warning.contains("mc-test"), "{warning}");
        assert!(warning.contains("inherit_forwarding = true"), "{warning}");
        assert!(warning.contains("client_only"), "{warning}");
    }
    assert_eq!(
        withheld_forwarding_warning(&policy, &ForwardingMode::None, "mc-test", &config),
        None
    );
}

#[test]
fn test_withheld_forwarding_warns_with_the_setting_that_restores_it() {
    let labels = make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.proxy_mode", "client_only"),
    ]);
    let policy = DockerProviderConfig::default();
    let config = container_server_config(&policy, "mc-test", &labels, ADDRESS).unwrap();

    for proxy_wide in [
        ForwardingMode::BungeeCord,
        ForwardingMode::BungeeGuard,
        ForwardingMode::Velocity,
    ] {
        let warning =
            withheld_forwarding_warning(&policy, &proxy_wide, "mc-test", &config).unwrap();
        assert!(warning.contains("mc-test"), "{warning}");
        assert!(warning.contains("inherit_forwarding = true"), "{warning}");
    }
}

#[test]
fn test_withheld_forwarding_stays_quiet_when_nothing_is_withheld() {
    let intercepted = make_labels(&[
        ("infrarust.enable", "true"),
        ("infrarust.proxy_mode", "client_only"),
    ]);
    let forwarding = make_labels(&[("infrarust.enable", "true")]);
    let closed = DockerProviderConfig::default();
    let inheriting = DockerProviderConfig {
        inherit_forwarding: true,
        ..Default::default()
    };
    let config = |policy, labels| container_server_config(policy, "mc-test", labels, ADDRESS);

    for (policy, proxy_wide, built) in [
        (
            &closed,
            ForwardingMode::None,
            config(&closed, &intercepted).unwrap(),
        ),
        (
            &inheriting,
            ForwardingMode::Velocity,
            config(&inheriting, &intercepted).unwrap(),
        ),
        (
            &closed,
            ForwardingMode::Velocity,
            config(&closed, &forwarding).unwrap(),
        ),
    ] {
        assert_eq!(
            withheld_forwarding_warning(policy, &proxy_wide, "mc-test", &built),
            None
        );
    }
}

#[test]
fn test_policy_settings_default_to_closed() {
    let parsed: DockerProviderConfig = toml::from_str("").unwrap();

    assert!(parsed.allowed_domains.is_empty());
    assert!(!parsed.inherit_forwarding);
}

async fn answer_with_no_containers(mut stream: impl AsyncRead + AsyncWrite + Unpin) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "the client closed before finishing its request");
        request.extend_from_slice(&chunk[..read]);
    }
    let request = String::from_utf8_lossy(&request);
    assert!(request.contains("/containers/json"), "{request}");
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
              Content-Length: 2\r\nConnection: close\r\n\r\n[]",
        )
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
}

fn provider_at(endpoint: String) -> DockerProvider {
    DockerProvider::new(
        &DockerProviderConfig {
            endpoint,
            ..Default::default()
        },
        ForwardingMode::None,
    )
}

#[tokio::test]
async fn test_provider_connects_to_the_configured_tcp_endpoint() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("tcp://{}", listener.local_addr().unwrap());
    let daemon = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        answer_with_no_containers(stream).await;
    });

    let configs = provider_at(endpoint).load_initial().await.unwrap();

    assert!(configs.is_empty());
    daemon.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn test_provider_connects_to_the_configured_unix_socket() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("docker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let endpoint = format!("unix://{}", socket.display());
    let daemon = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        answer_with_no_containers(stream).await;
    });

    let configs = provider_at(endpoint).load_initial().await.unwrap();

    assert!(configs.is_empty());
    daemon.await.unwrap();
}

#[tokio::test]
async fn test_provider_refuses_an_endpoint_it_cannot_honour() {
    for endpoint in ["/var/run/docker.sock", "ftp://daemon.lan:2375"] {
        let result = provider_at(endpoint.to_string()).load_initial().await;
        assert!(result.is_err(), "{endpoint}");
    }
}
