use infrarust_config::{ProxyMode, ServerConfig, ServerManagerConfig};

fn load_survival() -> ServerConfig {
    let toml_str = include_str!("fixtures/survival.toml");
    toml::from_str(toml_str).expect("failed to parse survival.toml")
}

fn load_creative() -> ServerConfig {
    let toml_str = include_str!("fixtures/creative.toml");
    toml::from_str(toml_str).expect("failed to parse creative.toml")
}

#[test]
fn test_parse_survival_domains() {
    let config = load_survival();

    assert_eq!(config.domains.len(), 2);
    assert_eq!(config.domains[0], "survival.mc.example.com");
    assert_eq!(config.domains[1], "*.survival.example.com");
}

#[test]
fn test_parse_survival_addresses() {
    let config = load_survival();

    assert_eq!(config.addresses.len(), 2);
    assert_eq!(config.addresses[0].host, "10.0.1.10");
    assert_eq!(config.addresses[0].port, 25565);
    assert_eq!(config.addresses[1].host, "10.0.1.11");
    assert_eq!(config.addresses[1].port, 25565);
}

#[test]
fn test_parse_survival_proxy_mode() {
    let config = load_survival();
    assert_eq!(config.proxy_mode, ProxyMode::Passthrough);
}

#[test]
fn test_parse_survival_motd() {
    let config = load_survival();

    let online = config
        .motd
        .online
        .as_ref()
        .expect("motd.online should be set");
    assert_eq!(online.text, "§aSurvival §7— §fBienvenue !");
    assert_eq!(online.favicon.as_deref(), Some("./icons/survival.png"));

    let sleeping = config
        .motd
        .sleeping
        .as_ref()
        .expect("motd.sleeping should be set");
    assert_eq!(sleeping.version_name.as_deref(), Some("Server Sleeping"));

    assert!(config.motd.starting.is_some());
    assert!(config.motd.offline.is_none());
}

#[test]
fn test_parse_survival_server_manager() {
    let config = load_survival();

    match config.server_manager {
        Some(ServerManagerConfig::Pterodactyl(ref ptero)) => {
            assert_eq!(ptero.api_url, "https://panel.example.com");
            assert_eq!(ptero.api_key, "ptlc_xxxxx");
            assert_eq!(ptero.server_id, "abc123");
        }
        other => panic!("expected Pterodactyl, got {other:?}"),
    }
}

#[test]
fn test_parse_survival_max_players() {
    let config = load_survival();
    assert_eq!(config.max_players, 100);
}

#[test]
fn test_parse_creative_proxy_mode() {
    let config = load_creative();
    assert_eq!(config.proxy_mode, ProxyMode::ClientOnly);
}

#[test]
fn test_parse_creative_server_manager_local() {
    let config = load_creative();

    match config.server_manager {
        Some(ServerManagerConfig::Local(ref local)) => {
            assert_eq!(local.command, "java");
            assert_eq!(
                local.working_dir.to_str().unwrap(),
                "/opt/minecraft/creative"
            );
            assert_eq!(local.args, vec!["-Xmx4G", "-jar", "server.jar", "nogui"]);
        }
        other => panic!("expected Local, got {other:?}"),
    }
}

#[test]
fn test_deny_unknown_fields() {
    let toml_str = r#"
        domains = ["test.example.com"]
        addresses = ["127.0.0.1:25565"]
        unknown_field = "oops"
    "#;
    let result: Result<ServerConfig, _> = toml::from_str(toml_str);
    assert!(result.is_err(), "unknown field should cause an error");
}
