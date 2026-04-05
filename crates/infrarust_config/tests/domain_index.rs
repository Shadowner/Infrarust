#![allow(clippy::unwrap_used, clippy::expect_used)]
use infrarust_config::{DomainIndex, DomainRewrite, MotdConfig, ProxyMode, ServerConfig};

fn make_config(id: &str, domains: &[&str], addr: &str) -> ServerConfig {
    ServerConfig {
        id: Some(id.to_string()),
        name: None,
        network: None,
        domains: domains.iter().map(ToString::to_string).collect(),
        addresses: vec![addr.parse().unwrap()],
        proxy_mode: ProxyMode::default(),
        forwarding_mode: None,
        send_proxy_protocol: false,
        domain_rewrite: DomainRewrite::default(),
        motd: MotdConfig::default(),
        server_manager: None,
        timeouts: None,
        max_players: 0,
        ip_filter: None,
        disconnect_message: None,
        limbo_handlers: vec![],
    }
}

#[test]
fn test_exact_match() {
    let configs = vec![make_config(
        "survival",
        &["survival.mc.example.com"],
        "10.0.1.10:25565",
    )];
    let index = DomainIndex::build(&configs);

    assert_eq!(index.resolve("survival.mc.example.com"), Some("survival"));
}

#[test]
fn test_exact_match_case_insensitive() {
    let configs = vec![make_config(
        "survival",
        &["survival.mc.example.com"],
        "10.0.1.10:25565",
    )];
    let index = DomainIndex::build(&configs);

    assert_eq!(index.resolve("Survival.MC.Example.COM"), Some("survival"));
}

#[test]
fn test_wildcard_match() {
    let configs = vec![make_config(
        "wild",
        &["*.mc.example.com"],
        "10.0.1.10:25565",
    )];
    let index = DomainIndex::build(&configs);

    assert_eq!(index.resolve("test.mc.example.com"), Some("wild"));
}

#[test]
fn test_exact_priority_over_wildcard() {
    let configs = vec![
        make_config("exact", &["a.b.com"], "10.0.1.10:25565"),
        make_config("wild", &["*.b.com"], "10.0.1.11:25565"),
    ];
    let index = DomainIndex::build(&configs);

    assert_eq!(index.resolve("a.b.com"), Some("exact"));
}

#[test]
fn test_no_match_returns_none() {
    let configs = vec![make_config(
        "survival",
        &["survival.mc.example.com"],
        "10.0.1.10:25565",
    )];
    let index = DomainIndex::build(&configs);

    assert_eq!(index.resolve("unknown.com"), None);
}

#[test]
fn test_empty_index() {
    let index = DomainIndex::build(&[]);

    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert_eq!(index.resolve("anything.com"), None);
}

#[test]
fn test_rebuild_after_config_change() {
    let configs_v1 = vec![make_config("old", &["old.example.com"], "10.0.1.10:25565")];
    let index_v1 = DomainIndex::build(&configs_v1);
    assert_eq!(index_v1.resolve("old.example.com"), Some("old"));

    let configs_v2 = vec![make_config("new", &["new.example.com"], "10.0.1.11:25565")];
    let index_v2 = DomainIndex::build(&configs_v2);
    assert_eq!(index_v2.resolve("old.example.com"), None);
    assert_eq!(index_v2.resolve("new.example.com"), Some("new"));
}

#[test]
fn test_multiple_wildcards_first_wins() {
    let configs = vec![
        make_config("specific", &["*.a.com"], "10.0.1.10:25565"),
        make_config("broad", &["*.com"], "10.0.1.11:25565"),
    ];
    let index = DomainIndex::build(&configs);

    assert_eq!(index.resolve("test.a.com"), Some("specific"));
}

#[test]
fn test_fml_marker_stripped() {
    let configs = vec![make_config(
        "survival",
        &["mc.example.com"],
        "10.0.1.10:25565",
    )];
    let index = DomainIndex::build(&configs);

    assert_eq!(index.resolve("mc.example.com\0FML"), Some("survival"));
    assert_eq!(index.resolve("mc.example.com\0FML2"), Some("survival"));
    assert_eq!(index.resolve("mc.example.com\0FML3"), Some("survival"));
}
