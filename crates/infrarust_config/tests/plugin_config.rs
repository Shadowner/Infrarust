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
    assert!(my_plugin.enabled);

    let analytics = &config.plugins["analytics"];
    assert!(analytics.path.is_none());
    assert!(analytics.permissions.is_empty());
    assert!(!analytics.enabled);
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

#[test]
fn test_plugin_deny_list_and_wasm_overrides() {
    let config: ProxyConfig = toml::from_str(
        r#"
        [plugins.locked]
        permissions = ["ban"]
        deny = ["player-write", "scheduler"]

        [plugins.locked.wasm]
        memory_limit_mb = 16
        "#,
    )
    .unwrap();
    let locked = &config.plugins["locked"];
    assert_eq!(locked.permissions, vec!["ban"]);
    assert_eq!(locked.deny, vec!["player-write", "scheduler"]);
    assert_eq!(
        locked.wasm.as_ref().and_then(|w| w.memory_limit_mb),
        Some(16)
    );

    let plain: ProxyConfig = toml::from_str("[plugins.plain]\n").unwrap();
    assert!(plain.plugins["plain"].deny.is_empty());
    assert!(plain.plugins["plain"].wasm.is_none());
}

#[test]
fn test_plugin_strict_capabilities_is_an_opt_in_flag() {
    let config: ProxyConfig = toml::from_str(
        r#"
        [plugins.locked]
        permissions = ["ban"]
        strict_capabilities = true

        [plugins.plain]
        "#,
    )
    .unwrap();
    assert!(config.plugins["locked"].strict_capabilities);
    assert!(!config.plugins["plain"].strict_capabilities);

    let not_a_bool = toml::from_str::<ProxyConfig>("[plugins.p]\nstrict_capabilities = \"yes\"\n")
        .expect_err("strict_capabilities is a boolean");
    assert!(
        not_a_bool.to_string().contains("strict_capabilities"),
        "{not_a_bool}"
    );
    let typo = toml::from_str::<ProxyConfig>("[plugins.p]\nstrict_capability = true\n")
        .expect_err("a misspelt key is refused, not ignored");
    assert!(typo.to_string().contains("strict_capability"), "{typo}");
}

#[test]
fn test_plugin_network_and_mounts_live_under_the_wasm_table() {
    let config: ProxyConfig = toml::from_str(
        r#"
        [plugins.libertybans]
        permissions = ["network", "filesystem-extended"]

        [plugins.libertybans.wasm.network]
        allow = ["127.0.0.1:5432", "db.internal:5432"]
        http = false

        [[plugins.libertybans.wasm.mounts]]
        host = "/srv/libertybans/shared"
        guest = "/shared"

        [[plugins.libertybans.wasm.mounts]]
        host = "/srv/libertybans/out"
        guest = "/out"
        read_only = false
        "#,
    )
    .unwrap();
    let plugin = &config.plugins["libertybans"];
    assert_eq!(plugin.permissions, vec!["network", "filesystem-extended"]);
    let wasm = plugin.wasm.as_ref().unwrap();
    let network = wasm.network.as_ref().unwrap();
    assert_eq!(network.allow.len(), 2);
    assert_eq!(network.allow[1].hostname(), Some("db.internal"));
    assert!(network.dns_enabled());
    assert!(!network.http);
    assert_eq!(wasm.mounts.len(), 2);
    assert!(wasm.mounts[0].read_only);
    assert!(!wasm.mounts[1].read_only);
    assert_eq!(wasm.mounts[1].guest_path().unwrap(), "/out");
}
