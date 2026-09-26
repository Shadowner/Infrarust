#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_config::{
    ProxyConfig, ServerConfig, validate_proxy_config, validate_server_config,
    validate_server_configs, validate_wasm_config, wasm_warnings,
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
fn test_proxy_webui_without_api_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for (api, ui, ok) in [
        (true, true, true),
        (true, false, true),
        (false, false, true),
        (false, true, false),
    ] {
        let config = proxy_from_toml(
            &format!("[web]\nenable_api = {api}\nenable_webui = {ui}"),
            dir.path(),
        );
        assert_eq!(
            validate_proxy_config(&config).is_ok(),
            ok,
            "enable_api = {api}, enable_webui = {ui}"
        );
    }
}

/// The dashboard cannot run without the API, so the minimal way to turn the
/// admin API off must not be the one combination that refuses to boot.
#[test]
fn test_proxy_disabling_the_api_alone_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml("[web]\nenable_api = false", dir.path());

    assert!(validate_proxy_config(&config).is_ok());
    assert!(
        !config
            .web
            .as_ref()
            .expect("a [web] section")
            .webui_enabled()
    );
}

#[test]
fn test_proxy_api_key_rules_match_startup() {
    let dir = tempfile::tempdir().unwrap();
    for (web, ok) in [
        ("bind = \"127.0.0.1:8080\"", true),
        ("bind = \"127.0.0.1:8080\"\napi_key = \"hunter2\"", false),
        (
            "bind = \"127.0.0.1:8080\"\napi_key = \"a-strong-enough-api-key\"",
            true,
        ),
        ("bind = \"0.0.0.0:8080\"", false),
        ("bind = \"0.0.0.0:8080\"\napi_key = \"\"", false),
        ("bind = \"0.0.0.0:8080\"\napi_key = \"CHANGE-ME\"", false),
        (
            "bind = \"0.0.0.0:8080\"\napi_key = \"a-strong-enough-api-key\"",
            true,
        ),
        (
            "enable_api = false\nbind = \"0.0.0.0:8080\"\napi_key = \"hunter2\"",
            true,
        ),
    ] {
        let config = proxy_from_toml(&format!("[web]\n{web}"), dir.path());
        assert_eq!(validate_proxy_config(&config).is_ok(), ok, "[web]\n{web}");
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
        api_key = "a-strong-enough-api-key"
    "#,
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_ok());
}

#[test]
fn test_proxy_zero_event_timeouts_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for key in [
        "handler_timeout",
        "slow_handler_threshold",
        "packet_handler_timeout",
        "transport_filter_timeout",
    ] {
        let config = proxy_from_toml(&format!("[events]\n{key} = \"0s\""), dir.path());
        let err = validate_proxy_config(&config).unwrap_err().to_string();
        assert!(err.contains(&format!("events.{key}")), "{err}");
    }
}

#[test]
fn test_proxy_ban_check_timeout_defaults_to_five_seconds_and_rejects_zero() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml("", dir.path());
    assert_eq!(config.ban.check_timeout, std::time::Duration::from_secs(5));
    let config = proxy_from_toml("[ban]\ncheck_timeout = \"250ms\"", dir.path());
    assert_eq!(
        config.ban.check_timeout,
        std::time::Duration::from_millis(250)
    );
    assert!(validate_proxy_config(&config).is_ok());
    let config = proxy_from_toml("[ban]\ncheck_timeout = \"0s\"", dir.path());
    let err = validate_proxy_config(&config).unwrap_err().to_string();
    assert!(err.contains("ban.check_timeout"), "{err}");
}

#[test]
fn test_proxy_default_wasm_section_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml("", dir.path());
    assert!(validate_proxy_config(&config).is_ok());
    assert!(wasm_warnings(&config).is_empty());
}

#[test]
fn test_proxy_zero_or_absurd_wasm_values_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for (line, key) in [
        ("epoch_tick = \"0s\"", "wasm.epoch_tick"),
        ("epoch_tick = \"2s\"", "wasm.epoch_tick"),
        ("memory_limit_mb = 0", "wasm.memory_limit_mb"),
        ("memory_limit_mb = 4097", "wasm.memory_limit_mb"),
        ("cpu_budget = \"0s\"", "wasm.cpu_budget"),
        ("cpu_budget = \"10ms\"", "wasm.cpu_budget"),
        ("cpu_budget = \"2h\"", "wasm.cpu_budget"),
        ("codec_cpu_budget = \"0s\"", "wasm.codec_cpu_budget"),
        ("host_call_timeout = \"0s\"", "wasm.host_call_timeout"),
        ("host_call_timeout = \"2h\"", "wasm.host_call_timeout"),
        ("max_call_duration = \"0s\"", "wasm.max_call_duration"),
        ("queue_capacity = 0", "wasm.queue_capacity"),
        ("queue_capacity = 2000000", "wasm.queue_capacity"),
    ] {
        let config = proxy_from_toml(&format!("[wasm]\n{line}"), dir.path());
        let err = validate_proxy_config(&config).expect_err(line).to_string();
        assert!(err.contains(key), "{line}: {err}");
    }
}

#[test]
fn test_proxy_out_of_range_wasm_recovery_values_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for (line, key) in [
        ("max_restarts = 1001", "wasm.recovery.max_restarts"),
        ("window = \"0s\"", "wasm.recovery.window"),
        ("window = \"2d\"", "wasm.recovery.window"),
        ("backoff_initial = \"0s\"", "wasm.recovery.backoff_initial"),
        ("backoff_max = \"0s\"", "wasm.recovery.backoff_max"),
        ("backoff_max = \"2d\"", "wasm.recovery.backoff_max"),
        (
            "backoff_initial = \"10m\"\nbackoff_max = \"1m\"",
            "wasm.recovery.backoff_initial",
        ),
    ] {
        let config = proxy_from_toml(&format!("[wasm.recovery]\n{line}"), dir.path());
        let err = validate_proxy_config(&config).expect_err(line).to_string();
        assert!(err.contains(key), "{line}: {err}");
    }
}

#[test]
fn test_proxy_in_range_wasm_recovery_values_are_valid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[wasm.recovery]\nmax_restarts = 0\nwindow = \"24h\"\nbackoff_initial = \"5m\"\nbackoff_max = \"5m\"",
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_ok());
}

#[test]
fn test_proxy_invalid_plugin_recovery_override_names_the_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[wasm.recovery]\nbackoff_max = \"1m\"\n\n[plugins.flaky.wasm.recovery]\nbackoff_initial = \"2m\"",
        dir.path(),
    );
    let err = validate_wasm_config(&config).unwrap_err().to_string();
    assert!(
        err.contains("plugins.flaky.wasm.recovery.backoff_initial"),
        "{err}"
    );
}

#[test]
fn test_proxy_invalid_plugin_wasm_override_names_the_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[plugins.chatty.wasm]\nqueue_capacity = 0\n\n[plugins.quiet.wasm]\nqueue_capacity = 8",
        dir.path(),
    );
    let err = validate_wasm_config(&config).unwrap_err().to_string();
    assert!(err.contains("plugins.chatty.wasm.queue_capacity"), "{err}");
}

#[test]
fn test_proxy_budget_shorter_than_the_epoch_tick_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[wasm]\nepoch_tick = \"100ms\"\n\n[plugins.p.wasm]\ncodec_cpu_budget = \"50ms\"",
        dir.path(),
    );
    let err = validate_wasm_config(&config).unwrap_err().to_string();
    assert!(err.contains("plugins.p.wasm.codec_cpu_budget"), "{err}");
}

#[test]
fn test_proxy_host_call_timeout_past_max_call_duration_is_not_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml("[plugins.p.wasm]\nmax_call_duration = \"5s\"", dir.path());
    assert!(validate_proxy_config(&config).is_ok());
    assert!(
        wasm_warnings(&config).is_empty(),
        "{:?}",
        wasm_warnings(&config)
    );
}

#[test]
fn test_proxy_cpu_budget_past_max_call_duration_is_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[plugins.p.wasm]\nmax_call_duration = \"2s\"\ncpu_budget = \"3s\"",
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_ok());
    let warnings = wasm_warnings(&config);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("plugins.p.wasm: cpu_budget"),
        "{warnings:?}"
    );
}

#[test]
fn test_bungeecord_channel_needs_an_intercepted_mode() {
    for mode in ["passthrough", "zero_copy", "server_only"] {
        let config = from_toml(&format!(
            r#"
            domains = ["mc.example.com"]
            addresses = ["127.0.0.1:25565"]
            proxy_mode = "{mode}"
            bungeecord_channel = true
        "#
        ));
        let err = validate_server_config(&config).unwrap_err().to_string();
        assert!(err.contains("bungeecord_channel"), "{mode}: {err}");
    }
    for mode in ["offline", "client_only"] {
        let config = from_toml(&format!(
            r#"
            addresses = ["127.0.0.1:25565"]
            proxy_mode = "{mode}"
            bungeecord_channel = true
        "#
        ));
        assert!(config.bungeecord_channel);
        assert!(validate_server_config(&config).is_ok(), "{mode}");
    }
}

#[test]
fn test_the_bungeecord_channel_is_off_by_default() {
    let server = from_toml(r#"addresses = ["127.0.0.1:25565"]"#);
    assert!(!server.bungeecord_channel);
    let dir = tempfile::tempdir().unwrap();
    let proxy = proxy_from_toml("", dir.path());
    assert!(!proxy.plugin_messaging.bungeecord);
    let permissions = &proxy.plugin_messaging.bungeecord_permissions;
    assert!(permissions.connect && permissions.player_list && permissions.forward);
    assert!(!permissions.connect_other && !permissions.message && !permissions.kick_player);
}

#[test]
fn test_plugin_messaging_section_parses() {
    let dir = tempfile::tempdir().unwrap();
    let proxy = proxy_from_toml(
        r#"
        [plugin_messaging]
        bungeecord = true

        [plugin_messaging.bungeecord_permissions]
        message = true
        KickPlayer = true
    "#,
        dir.path(),
    );
    assert!(proxy.plugin_messaging.bungeecord);
    assert!(proxy.plugin_messaging.bungeecord_permissions.message);
    assert!(proxy.plugin_messaging.bungeecord_permissions.kick_player);
    assert!(validate_proxy_config(&proxy).is_ok());
}

#[test]
fn test_the_moved_forwarding_channel_keys_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let proxy = proxy_from_toml(
        r#"
        [forwarding]
        mode = "none"
        bungeecord_channel = true

        [forwarding.channel_permissions]
        message = true
    "#,
        dir.path(),
    );
    let forwarding = proxy.forwarding.as_ref().unwrap();
    assert!(forwarding.has_moved_channel_keys());
    assert!(!proxy.plugin_messaging.bungeecord);
    assert!(validate_proxy_config(&proxy).is_ok());
}

#[test]
fn test_proxy_wasm_mount_guest_paths_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let mounts = |entries: &[(&str, &str)]| {
        entries
            .iter()
            .map(|(host, guest)| {
                format!("[[plugins.p.wasm.mounts]]\nhost = \"{host}\"\nguest = \"{guest}\"\n")
            })
            .collect::<String>()
    };
    for (entries, expected) in [
        (
            vec![("/srv/a", "shared")],
            "plugins.p.wasm.mounts: guest path \"shared\" must be absolute",
        ),
        (
            vec![("/srv/a", "/")],
            "plugins.p.wasm.mounts: guest path \"/\" is the plugin data directory; mount somewhere below it",
        ),
        (
            vec![("/srv/a", "/a/../b")],
            "plugins.p.wasm.mounts: guest path \"/a/../b\" must not contain `.` or `..`",
        ),
        (
            vec![("/srv/a", "/shared"), ("/srv/b", "/shared/")],
            "plugins.p.wasm.mounts: guest path \"/shared\" is mounted twice",
        ),
        (
            vec![("/srv/a", "/shared"), ("/srv/b", "/shared/sub")],
            "plugins.p.wasm.mounts: guest paths \"/shared\" and \"/shared/sub\" overlap",
        ),
        (
            vec![("/srv/a", "/shared/sub"), ("/srv/b", "/shared")],
            "plugins.p.wasm.mounts: guest paths \"/shared\" and \"/shared/sub\" overlap",
        ),
        (
            vec![("", "/shared")],
            "plugins.p.wasm.mounts: the host path of \"/shared\" must not be empty",
        ),
    ] {
        let config = proxy_from_toml(&mounts(&entries), dir.path());
        let err = validate_proxy_config(&config)
            .expect_err(expected)
            .to_string();
        assert_eq!(err, format!("validation error: {expected}"));
    }

    let fine = proxy_from_toml(
        &mounts(&[
            ("/srv/a", "/shared"),
            ("/srv/b", "/shared-two"),
            ("/srv/c", "/x/y"),
        ]),
        dir.path(),
    );
    assert!(validate_proxy_config(&fine).is_ok());
}

#[test]
fn test_proxy_wasm_mount_hosts_are_not_checked_by_the_document_validation() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[[plugins.p.wasm.mounts]]\nhost = \"/definitely/not/here\"\nguest = \"/shared\"\n",
        dir.path(),
    );
    assert!(
        validate_proxy_config(&config).is_ok(),
        "a missing host directory fails the plugin load, not the proxy"
    );
}
