#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_config::{
    ConfigError, ForwardingMode, ProxyConfig, ProxyValidationError, ServerConfig,
    ServerValidationError, WasmValidationError, validate_proxy_config, validate_server_config,
    validate_server_configs, validate_server_forwarding, validate_wasm_config, wasm_warnings,
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
fn full_mode_is_reserved_and_rejected() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "full"
    "#,
    );
    let error = validate_server_config(&config).unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::Server {
                reason: ServerValidationError::FullModeReserved,
                ..
            }
        ),
        "{error}"
    );
    let error = error.to_string();
    assert!(error.contains("full"), "{error}");
    assert!(error.contains("not implemented"), "{error}");
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
    let err = validate_proxy_config(&config).unwrap_err();
    assert!(
        matches!(
            err,
            ConfigError::Proxy(ProxyValidationError::ZeroDuration {
                key: "ban.check_timeout"
            })
        ),
        "{err}"
    );
    assert!(err.to_string().contains("ban.check_timeout"), "{err}");
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
        (
            "epoch_tick = \"20ms\"\ncpu_budget = \"10ms\"",
            "wasm.cpu_budget",
        ),
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
fn test_proxy_wasm_cache_dir_defaults_outside_plugins_dir() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml("", dir.path());
    assert_eq!(config.wasm.cache_dir, std::path::Path::new("./cache/wasm"));
    assert!(validate_proxy_config(&config).is_ok());
}

#[test]
fn test_proxy_wasm_cache_dir_inside_plugins_dir_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for (plugins, cache) in [
        ("./plugins", "./plugins/.cache"),
        ("./plugins", "plugins"),
        ("plugins", "./plugins/../plugins/aot"),
        (
            "/srv/infrarust/plugins",
            "/srv/infrarust/plugins/wasm-cache",
        ),
        ("/srv/infrarust/plugins/", "/srv/infrarust/plugins"),
    ] {
        let toml = format!("plugins_dir = {plugins:?}\n[wasm]\ncache_dir = {cache:?}\n");
        let config = proxy_from_toml(&toml, dir.path());
        let err = validate_proxy_config(&config).expect_err(&toml).to_string();
        assert!(err.contains("wasm.cache_dir"), "{toml}: {err}");
        assert!(err.contains("outside plugins_dir"), "{toml}: {err}");
    }
    let config = proxy_from_toml("[wasm]\ncache_dir = \"\"\n", dir.path());
    let err = validate_proxy_config(&config)
        .expect_err("empty")
        .to_string();
    assert!(err.contains("wasm.cache_dir must not be empty"), "{err}");
}

#[test]
fn test_proxy_wasm_cache_dir_next_to_plugins_dir_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    for (plugins, cache) in [
        ("./plugins", "./cache/wasm"),
        ("./plugins", "./plugins-cache"),
        ("./plugins", "./plugins/../cache"),
        ("/srv/infrarust/plugins", "/srv/infrarust/cache/wasm"),
        ("/srv/infrarust/plugins", "/var/cache/infrarust"),
    ] {
        let toml = format!("plugins_dir = {plugins:?}\n[wasm]\ncache_dir = {cache:?}\n");
        let config = proxy_from_toml(&toml, dir.path());
        assert!(validate_proxy_config(&config).is_ok(), "{toml}");
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
fn test_proxy_out_of_range_wasm_quotas_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for (line, key) in [
        ("event_listeners = 0", "wasm.quotas.event_listeners"),
        ("commands = 0", "wasm.quotas.commands"),
        ("scheduled_tasks = 0", "wasm.quotas.scheduled_tasks"),
        ("plugin_channels = 0", "wasm.quotas.plugin_channels"),
        ("codec_filters = 0", "wasm.quotas.codec_filters"),
        ("limbo_handlers = 0", "wasm.quotas.limbo_handlers"),
        ("event_listeners = 1048577", "wasm.quotas.event_listeners"),
        ("scheduled_tasks = 2000000", "wasm.quotas.scheduled_tasks"),
    ] {
        let config = proxy_from_toml(&format!("[wasm.quotas]\n{line}"), dir.path());
        let err = validate_proxy_config(&config).expect_err(line).to_string();
        assert!(err.contains(key), "{line}: {err}");
    }
}

#[test]
fn test_proxy_in_range_wasm_quotas_are_valid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[wasm.quotas]\nevent_listeners = 1\ncommands = 1048576\n\n[plugins.p.wasm.quotas]\ncodec_filters = 1",
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_ok());
}

#[test]
fn test_proxy_invalid_plugin_quota_override_names_the_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[plugins.greedy.wasm.quotas]\ncommands = 0\n\n[plugins.modest.wasm.quotas]\ncommands = 8",
        dir.path(),
    );
    let err = validate_wasm_config(&config).unwrap_err();
    assert!(
        matches!(
            &err,
            ConfigError::Wasm(WasmValidationError::QuotaOutOfRange { scope, key: "commands", value: 0 })
                if scope == "plugins.greedy.wasm"
        ),
        "{err}"
    );
    assert!(
        err.to_string()
            .contains("plugins.greedy.wasm.quotas.commands"),
        "{err}"
    );
}

#[test]
fn test_proxy_out_of_range_wasm_codec_quarantine_values_are_invalid() {
    let dir = tempfile::tempdir().unwrap();
    for (line, key) in [
        ("faults = 1000001", "wasm.codec_quarantine.faults"),
        ("window = \"0s\"", "wasm.codec_quarantine.window"),
        ("window = \"2d\"", "wasm.codec_quarantine.window"),
        (
            "backoff_initial = \"0s\"",
            "wasm.codec_quarantine.backoff_initial",
        ),
        ("backoff_max = \"2d\"", "wasm.codec_quarantine.backoff_max"),
        (
            "backoff_initial = \"10m\"\nbackoff_max = \"1m\"",
            "wasm.codec_quarantine.backoff_initial",
        ),
    ] {
        let config = proxy_from_toml(&format!("[wasm.codec_quarantine]\n{line}"), dir.path());
        let err = validate_proxy_config(&config).expect_err(line).to_string();
        assert!(err.contains(key), "{line}: {err}");
    }
}

#[test]
fn test_proxy_disabled_codec_quarantine_and_a_plugin_override_are_valid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[wasm]\nepoch_tick = \"1ms\"\ncodec_cpu_budget = \"1ms\"\n\n[wasm.codec_quarantine]\nfaults = 0\n\n[plugins.anticheat.wasm.codec_quarantine]\nfaults = 3\nbackoff_initial = \"1m\"",
        dir.path(),
    );
    assert!(validate_proxy_config(&config).is_ok());
    let err = validate_wasm_config(&proxy_from_toml(
        "[plugins.anticheat.wasm.codec_quarantine]\nbackoff_initial = \"10m\"",
        dir.path(),
    ))
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("plugins.anticheat.wasm.codec_quarantine.backoff_initial"),
        "{err}"
    );
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
    let err = validate_wasm_config(&config).unwrap_err();
    assert!(
        matches!(
            &err,
            ConfigError::Wasm(WasmValidationError::QueueCapacityOutOfRange { scope, value: 0 })
                if scope == "plugins.chatty.wasm"
        ),
        "{err}"
    );
    assert!(
        err.to_string()
            .contains("plugins.chatty.wasm.queue_capacity"),
        "{err}"
    );
}

#[test]
fn test_proxy_budget_shorter_than_the_epoch_tick_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "[wasm]\nepoch_tick = \"100ms\"\n\n[plugins.p.wasm]\ncpu_budget = \"50ms\"",
        dir.path(),
    );
    let err = validate_wasm_config(&config).unwrap_err().to_string();
    assert!(err.contains("plugins.p.wasm.cpu_budget"), "{err}");
}

#[test]
fn test_proxy_codec_budget_shorter_than_the_epoch_tick_is_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml("[wasm]\nepoch_tick = \"50ms\"", dir.path());
    let warnings = validate_wasm_config(&config).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("wasm: codec_cpu_budget (5ms) is shorter than wasm.epoch_tick (50ms)"),
        "{warnings:?}"
    );
    let config = proxy_from_toml("", dir.path());
    assert!(validate_wasm_config(&config).unwrap().is_empty());
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
        let err = validate_server_config(&config).unwrap_err();
        assert!(
            matches!(
                &err,
                ConfigError::Server {
                    id,
                    reason: ServerValidationError::BungeecordChannelNotIntercepted { .. }
                } if id == "unknown"
            ),
            "{mode}: {err}"
        );
        assert!(
            err.to_string().contains("bungeecord_channel"),
            "{mode}: {err}"
        );
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
        assert_eq!(err, expected);
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

#[test]
fn velocity_forwarding_on_a_passthrough_server_is_invalid() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "passthrough"
        forwarding_mode = "velocity"
    "#,
    );
    let error = validate_server_config(&config).unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::Server {
                reason: ServerValidationError::VelocityOnForwardingServer {
                    mode_source: "its forwarding_mode",
                    ..
                },
                ..
            }
        ),
        "{error}"
    );
    let error = error.to_string();
    assert!(error.contains("velocity"), "{error}");
    assert!(error.to_lowercase().contains("passthrough"), "{error}");
}

#[test]
fn a_proxy_wide_velocity_default_is_rejected_for_a_forwarding_server() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "server_only"
    "#,
    );
    let error = validate_server_forwarding(&config, ForwardingMode::Velocity).unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::Server {
                reason: ServerValidationError::VelocityOnForwardingServer {
                    mode_source: "the proxy-wide [forwarding] mode",
                    ..
                },
                ..
            }
        ),
        "{error}"
    );
    assert!(error.to_string().contains("velocity"), "{error}");
    assert!(validate_server_forwarding(&config, ForwardingMode::BungeeCord).is_ok());
}

#[test]
fn a_server_override_can_opt_out_of_a_proxy_wide_velocity_default() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "passthrough"
        forwarding_mode = "none"
    "#,
    );
    assert!(validate_server_forwarding(&config, ForwardingMode::Velocity).is_ok());
}

#[test]
fn velocity_forwarding_on_an_intercepted_server_is_valid() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["127.0.0.1:25565"]
        proxy_mode = "offline"
        forwarding_mode = "velocity"
    "#,
    );
    assert!(validate_server_config(&config).is_ok());
    assert!(validate_server_forwarding(&config, ForwardingMode::Velocity).is_ok());
}

#[test]
fn test_server_validation_returns_its_warnings() {
    let config = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565", "10.0.0.2:25565"]
        slow_start = "45s"
    "#,
    );
    let warnings = validate_server_config(&config).unwrap();
    assert_eq!(warnings, infrarust_config::balance_warnings(&config));
    assert_eq!(warnings.len(), 2, "{warnings:?}");

    let quiet = from_toml(
        r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565"]
    "#,
    );
    assert!(validate_server_config(&quiet).unwrap().is_empty());
}

#[test]
fn test_proxy_validation_returns_its_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let config = proxy_from_toml(
        "plugins_dir = \"/definitely/not/here\"\n\n[plugins.p]\n\n[plugins.p.wasm]\nmax_call_duration = \"2s\"\ncpu_budget = \"3s\"\n\n[forwarding]\nbungeecord_channel = true\n",
        dir.path(),
    );
    let warnings = validate_proxy_config(&config).unwrap();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings[0].starts_with("plugins.p.wasm: cpu_budget"),
        "{warnings:?}"
    );
    assert!(
        warnings[1].contains("[forwarding] bungeecord_channel"),
        "{warnings:?}"
    );
    assert!(
        warnings[2].contains("plugins_dir /definitely/not/here does not exist"),
        "{warnings:?}"
    );

    let document_warnings = infrarust_config::validate_proxy_document(&config).unwrap();
    assert_eq!(document_warnings, wasm_warnings(&config));

    let quiet = proxy_from_toml("", dir.path());
    assert!(validate_proxy_config(&quiet).unwrap().is_empty());
}
