//! Validation helpers for configuration structs.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use crate::error::ConfigError;
use crate::proxy::ProxyConfig;
use crate::server::ServerConfig;
use crate::types::{BalanceStrategy, WasmLimits};

/// Validates a single server configuration.
///
/// Checks:
/// - Forwarding modes (Passthrough, ZeroCopy, ServerOnly) have at least one domain
/// - Forwarding modes cannot belong to a network (no server switching support)
/// - At least one address is defined
/// - No empty domain strings
/// - The effective id matches `[a-z0-9_.-]+` (used as ServerId in routing/telemetry)
/// - `name` (if set) matches `[a-z0-9_-]+`
/// - `network` (if set) matches `[a-z0-9_-]+`
///
/// # Errors
///
/// Returns [`ConfigError::NoDomains`] if a forwarding-mode server has no domains,
/// [`ConfigError::NoAddresses`] if no addresses are defined, or
/// [`ConfigError::Validation`] if any domain string is empty or id/name/network are invalid.
pub fn validate_server_config(config: &ServerConfig) -> Result<(), ConfigError> {
    let id = config.effective_id();
    validate_effective_id(&id)?;

    if config.proxy_mode.is_forwarding() {
        if config.domains.is_empty() {
            return Err(ConfigError::NoDomains {
                id: id.clone(),
                proxy_mode: config.proxy_mode,
            });
        }
        if config.network.is_some() {
            return Err(ConfigError::Validation(format!(
                "server '{id}' uses {:?} mode which cannot belong to a network \
                 (forwarding modes don't support server switching)",
                config.proxy_mode
            )));
        }
    }

    if config.addresses.is_empty() {
        return Err(ConfigError::NoAddresses { id });
    }

    for domain in &config.domains {
        if domain.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "server config {id} has an empty domain"
            )));
        }
    }

    if let Some(name) = &config.name {
        validate_identifier(name, "name", &id)?;
    }

    if let Some(network) = &config.network {
        validate_identifier(network, "network", &id)?;
    }

    if !config.slow_start_aggression.is_finite() || config.slow_start_aggression <= 0.0 {
        return Err(ConfigError::Validation(format!(
            "server '{id}': slow_start_aggression must be a finite number > 0 (got {})",
            config.slow_start_aggression
        )));
    }

    for warning in balance_warnings(config) {
        tracing::warn!(server = %id, "{warning}");
    }

    #[cfg(not(target_os = "linux"))]
    if config.proxy_mode == crate::types::ProxyMode::ZeroCopy {
        tracing::warn!(
            server = %id,
            "proxy_mode = zero_copy is only supported on Linux"
        );
    }

    Ok(())
}

/// Returns the load-balancing configuration warnings for a server config.
///
/// Pure so it can be unit-tested; `validate_server_config` logs each entry.
pub fn balance_warnings(config: &ServerConfig) -> Vec<String> {
    let mut warnings = Vec::new();

    for wa in &config.addresses {
        if wa.weight == 0 {
            warnings.push(format!(
                "address {} has weight 0, which is treated as weight 1; \
                 remove the address from the list to drain it",
                wa.address
            ));
        }
    }

    if config.balance == BalanceStrategy::FirstAvailable {
        if config.addresses.len() > 1 {
            warnings.push(format!(
                "{} addresses configured with balance = \"first_available\": all traffic \
                 goes to the first healthy address; consider balance = \"least_conn\"",
                config.addresses.len()
            ));
        }
        if config.slow_start.is_some() {
            warnings
                .push("slow_start has no effect with balance = \"first_available\"".to_string());
        }
    }

    warnings
}

fn validate_identifier(value: &str, field: &str, server_id: &str) -> Result<(), ConfigError> {
    validate_ident_chars(value, field, server_id, false)
}

fn validate_effective_id(id: &str) -> Result<(), ConfigError> {
    validate_ident_chars(id, "id", id, true)
}

fn validate_ident_chars(
    value: &str,
    field: &str,
    server_id: &str,
    allow_dot: bool,
) -> Result<(), ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::Validation(format!(
            "server '{server_id}': {field} must not be empty"
        )));
    }
    if value.len() > 64 {
        return Err(ConfigError::Validation(format!(
            "server '{server_id}': {field} must be at most 64 characters"
        )));
    }
    if !value.bytes().all(|b| {
        b.is_ascii_lowercase()
            || b.is_ascii_digit()
            || b == b'_'
            || b == b'-'
            || (allow_dot && b == b'.')
    }) {
        let allowed = if allow_dot {
            "a-z, 0-9, _, -, ."
        } else {
            "a-z, 0-9, _, -"
        };
        return Err(ConfigError::Validation(format!(
            "server '{server_id}': {field} '{value}' contains invalid characters (allowed: {allowed})"
        )));
    }
    Ok(())
}

/// Validates a batch of server configurations for duplicate or invalid IDs.
///
/// Runs after providers assign filename-derived ids, so it also
/// charset-validates ids that `validate_server_config` saw before assignment.
///
/// # Errors
///
/// Returns [`ConfigError::DuplicateId`] if two or more configs share the
/// same `effective_id()`, or [`ConfigError::Validation`] if an effective id
/// has an invalid charset.
pub fn validate_server_configs(configs: &[ServerConfig]) -> Result<(), ConfigError> {
    let mut seen = HashSet::with_capacity(configs.len());
    for config in configs {
        let id = config.effective_id();
        validate_effective_id(&id)?;
        if !seen.insert(id.clone()) {
            return Err(ConfigError::DuplicateId(id));
        }
    }
    Ok(())
}

/// Validates the global proxy configuration the proxy is about to run on.
///
/// Checks:
/// - `servers_dir` exists on disk
/// - `connect_timeout`, rate-limit windows and `docker.poll_interval` are non-zero
/// - `telemetry.protocol` is `"grpc"` or `"http"`
/// - `web.bind` is a parseable `host:port` and does not collide with `bind`
/// - `web.api_key` is one [`WebConfig::resolve_api_key`](crate::WebConfig::resolve_api_key) accepts
///
/// Logs a warning when plugins are configured but `plugins_dir` is missing
/// (not fatal: built-in plugins don't need the directory).
///
/// # Errors
///
/// Returns [`ConfigError::DirectoryNotFound`] if `servers_dir` does not
/// exist or is not a directory, or [`ConfigError::Validation`] for any
/// other failed check.
pub fn validate_proxy_config(config: &ProxyConfig) -> Result<(), ConfigError> {
    if !config.servers_dir.is_dir() {
        return Err(ConfigError::DirectoryNotFound(config.servers_dir.clone()));
    }

    validate_proxy_document(config)?;

    if !config.plugins.is_empty() && !config.plugins_dir.is_dir() {
        tracing::warn!(
            plugins_dir = %config.plugins_dir.display(),
            "plugins are configured but plugins_dir does not exist \
             (only built-in plugins will be available)"
        );
    }

    Ok(())
}

/// Validates everything in a proxy configuration document except where its
/// directories point.
///
/// A running proxy may have been started with `--servers-dir` or
/// `--plugins-dir`, so the paths a document carries do not decide whether it
/// boots; [`validate_proxy_config`] checks them against the process that is
/// about to use them.
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] for any failed check.
pub fn validate_proxy_document(config: &ProxyConfig) -> Result<(), ConfigError> {
    if config.connect_timeout.is_zero() {
        return Err(ConfigError::Validation(
            "connect_timeout must be greater than zero".to_string(),
        ));
    }

    for (key, value) in [
        ("events.handler_timeout", config.events.handler_timeout),
        (
            "events.slow_handler_threshold",
            config.events.slow_handler_threshold,
        ),
        (
            "events.packet_handler_timeout",
            config.events.packet_handler_timeout,
        ),
        (
            "events.disconnect_deadline",
            config.events.disconnect_deadline,
        ),
    ] {
        if value.is_zero() {
            return Err(ConfigError::Validation(format!(
                "{key} must be greater than zero"
            )));
        }
    }

    validate_wasm_config(config)?;

    if config.rate_limit.enabled {
        if config.rate_limit.window.is_zero() {
            return Err(ConfigError::Validation(
                "rate_limit.window must be greater than zero".to_string(),
            ));
        }
        if config.rate_limit.status_window.is_zero() {
            return Err(ConfigError::Validation(
                "rate_limit.status_window must be greater than zero".to_string(),
            ));
        }
    }

    if let Some(docker) = &config.docker
        && docker.poll_interval.is_zero()
    {
        return Err(ConfigError::Validation(
            "docker.poll_interval must be greater than zero".to_string(),
        ));
    }

    if let Some(telemetry) = &config.telemetry
        && !matches!(telemetry.protocol.as_str(), "grpc" | "http")
    {
        return Err(ConfigError::Validation(format!(
            "telemetry.protocol must be \"grpc\" or \"http\" (got '{}')",
            telemetry.protocol
        )));
    }

    if let Some(web) = &config.web {
        validate_web_bind(&web.bind, config.bind)?;
        if web.enable_webui == Some(true) && !web.enable_api {
            return Err(ConfigError::Validation(
                "web.enable_webui requires web.enable_api: the dashboard is served by the \
                 admin API and cannot run without it"
                    .to_string(),
            ));
        }
        if web.enable_api {
            web.check_api_key().map_err(ConfigError::Validation)?;
        }
    }

    Ok(())
}

/// `web.bind` accepts a socket address or `hostname:port`
/// (the hostname is resolved when the listener binds).
fn validate_web_bind(bind: &str, proxy_bind: SocketAddr) -> Result<(), ConfigError> {
    let (web_ip, port) = if let Ok(addr) = bind.parse::<SocketAddr>() {
        (Some(addr.ip()), addr.port())
    } else {
        let Some((host, port_str)) = bind.rsplit_once(':') else {
            return Err(ConfigError::Validation(format!(
                "web.bind '{bind}' is not a valid bind address (expected host:port)"
            )));
        };
        let Ok(port) = port_str.parse::<u16>() else {
            return Err(ConfigError::Validation(format!(
                "web.bind '{bind}' has an invalid port '{port_str}'"
            )));
        };
        let host = host.trim_matches(['[', ']']);
        if host.is_empty() || host.chars().any(char::is_whitespace) {
            return Err(ConfigError::Validation(format!(
                "web.bind '{bind}' has an invalid host"
            )));
        }
        let ip = host.parse::<IpAddr>().ok().or_else(|| {
            host.eq_ignore_ascii_case("localhost")
                .then_some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        });
        (ip, port)
    };

    if port == proxy_bind.port() {
        let collides = proxy_bind.ip().is_unspecified()
            || web_ip.is_some_and(|ip| ip.is_unspecified() || ip == proxy_bind.ip());
        if collides {
            return Err(ConfigError::Validation(format!(
                "web.bind '{bind}' collides with the proxy bind address '{proxy_bind}'"
            )));
        }
    }

    Ok(())
}

const WASM_MIN_EPOCH_TICK: Duration = Duration::from_millis(1);
const WASM_MAX_EPOCH_TICK: Duration = Duration::from_secs(1);
const WASM_MAX_DURATION: Duration = Duration::from_secs(3600);
const WASM_MAX_MEMORY_MB: u32 = 4096;
const WASM_MAX_QUEUE_CAPACITY: usize = 1 << 20;

pub fn validate_wasm_config(config: &ProxyConfig) -> Result<(), ConfigError> {
    let tick = config.wasm.epoch_tick;
    if !(WASM_MIN_EPOCH_TICK..=WASM_MAX_EPOCH_TICK).contains(&tick) {
        return Err(ConfigError::Validation(format!(
            "wasm.epoch_tick must be between {} and {} (got {})",
            humantime::format_duration(WASM_MIN_EPOCH_TICK),
            humantime::format_duration(WASM_MAX_EPOCH_TICK),
            humantime::format_duration(tick)
        )));
    }
    validate_wasm_limits("wasm", &config.wasm.limits(), tick)?;
    let mut ids: Vec<&String> = config.plugins.keys().collect();
    ids.sort();
    for id in ids {
        let overrides = config.plugins[id].wasm.as_ref();
        if overrides.is_some() {
            let limits = config.wasm.limits_for(overrides);
            validate_wasm_limits(&format!("plugins.{id}.wasm"), &limits, tick)?;
        }
    }
    for warning in wasm_warnings(config) {
        tracing::warn!("{warning}");
    }
    Ok(())
}

pub fn wasm_warnings(config: &ProxyConfig) -> Vec<String> {
    let mut scopes = vec![("wasm".to_string(), config.wasm.limits())];
    let mut ids: Vec<&String> = config.plugins.keys().collect();
    ids.sort();
    for id in ids {
        let overrides = config.plugins[id].wasm.as_ref();
        if overrides.is_some() {
            scopes.push((
                format!("plugins.{id}.wasm"),
                config.wasm.limits_for(overrides),
            ));
        }
    }
    let mut warnings = Vec::new();
    for (scope, limits) in scopes {
        if limits.host_call_timeout > limits.max_call_duration {
            warnings.push(format!(
                "{scope}: host_call_timeout ({}) is longer than max_call_duration ({}); \
                 a slow host call will be cut off by max_call_duration and disable the plugin \
                 instead of returning a service error to it",
                humantime::format_duration(limits.host_call_timeout),
                humantime::format_duration(limits.max_call_duration)
            ));
        }
        if limits.cpu_budget > limits.max_call_duration {
            warnings.push(format!(
                "{scope}: cpu_budget ({}) is longer than max_call_duration ({}); \
                 max_call_duration stops a busy guest call first",
                humantime::format_duration(limits.cpu_budget),
                humantime::format_duration(limits.max_call_duration)
            ));
        }
    }
    warnings
}

fn validate_wasm_limits(
    scope: &str,
    limits: &WasmLimits,
    tick: Duration,
) -> Result<(), ConfigError> {
    if !(1..=WASM_MAX_MEMORY_MB).contains(&limits.memory_limit_mb) {
        return Err(ConfigError::Validation(format!(
            "{scope}.memory_limit_mb must be between 1 and {WASM_MAX_MEMORY_MB} (got {})",
            limits.memory_limit_mb
        )));
    }
    for (key, budget) in [
        ("cpu_budget", limits.cpu_budget),
        ("codec_cpu_budget", limits.codec_cpu_budget),
    ] {
        if budget < tick || budget > WASM_MAX_DURATION {
            return Err(ConfigError::Validation(format!(
                "{scope}.{key} must be between wasm.epoch_tick ({}) and {} (got {})",
                humantime::format_duration(tick),
                humantime::format_duration(WASM_MAX_DURATION),
                humantime::format_duration(budget)
            )));
        }
    }
    for (key, value) in [
        ("host_call_timeout", limits.host_call_timeout),
        ("max_call_duration", limits.max_call_duration),
    ] {
        if value.is_zero() || value > WASM_MAX_DURATION {
            return Err(ConfigError::Validation(format!(
                "{scope}.{key} must be greater than zero and at most {} (got {})",
                humantime::format_duration(WASM_MAX_DURATION),
                humantime::format_duration(value)
            )));
        }
    }
    if !(1..=WASM_MAX_QUEUE_CAPACITY).contains(&limits.queue_capacity) {
        return Err(ConfigError::Validation(format!(
            "{scope}.queue_capacity must be between 1 and {WASM_MAX_QUEUE_CAPACITY} (got {})",
            limits.queue_capacity
        )));
    }
    Ok(())
}
