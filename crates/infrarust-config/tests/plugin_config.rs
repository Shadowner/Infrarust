#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_config::proxy::ProxyConfig;
use infrarust_config::server::ServerConfig;

#[test]
fn test_parse_limbo_handlers() {
    let toml = include_str!("fixtures/with_limbo.toml");
    let config: ServerConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.limbo_handlers, vec!["auth", "antibot"]);
}

#[test]
fn test_parse_plugins_section() {
    let toml = include_str!("fixtures/with_plugins.toml");
    let config: ProxyConfig = toml::from_str(toml).unwrap();

    assert_eq!(config.plugins.len(), 2);

    let my_plugin = &config.plugins["my_plugin"];
    assert_eq!(my_plugin.path.as_deref(), Some("/opt/plugins/my_plugin.so"));
    assert_eq!(my_plugin.permissions, vec!["admin.kick", "admin.ban"]);
    assert_eq!(my_plugin.enabled, Some(true));

    let analytics = &config.plugins["analytics"];
    assert!(analytics.path.is_none());
    assert!(analytics.permissions.is_empty());
    assert_eq!(analytics.enabled, Some(false));
}

#[test]
fn test_empty_defaults() {
    let toml = r#"
        domains = ["test.example.com"]
        addresses = ["127.0.0.1:25565"]
    "#;
    let config: ServerConfig = toml::from_str(toml).unwrap();
    assert!(config.limbo_handlers.is_empty());

    let proxy_toml = r#"
        bind = "0.0.0.0:25565"
    "#;
    let proxy_config: ProxyConfig = toml::from_str(proxy_toml).unwrap();
    assert!(proxy_config.plugins.is_empty());
}
