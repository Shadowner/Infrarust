//! Validation helpers for configuration structs.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::error::{ConfigError, ProxyValidationError, ServerValidationError, WasmValidationError};
use crate::proxy::ProxyConfig;
use crate::server::ServerConfig;
use crate::types::{
    BalanceStrategy, ForwardingConfig, ForwardingMode, PluginWasmConfig, WasmLimits,
    WasmQuotasConfig, WasmRecoveryConfig,
};

fn server_error(id: &str, reason: ServerValidationError) -> ConfigError {
    ConfigError::Server {
        id: id.to_string(),
        reason,
    }
}

pub fn validate_server_config(config: &ServerConfig) -> Result<Vec<String>, ConfigError> {
    let id = config.effective_id();
    validate_effective_id(&id)?;

    if config.proxy_mode == crate::types::ProxyMode::Full {
        return Err(server_error(&id, ServerValidationError::FullModeReserved));
    }

    if config.proxy_mode.is_forwarding() {
        if config.domains.is_empty() {
            return Err(ConfigError::NoDomains {
                id: id.clone(),
                proxy_mode: config.proxy_mode,
            });
        }
        if config.network.is_some() {
            return Err(server_error(
                &id,
                ServerValidationError::ForwardingInNetwork {
                    proxy_mode: config.proxy_mode,
                },
            ));
        }
        if config.bungeecord_channel {
            return Err(server_error(
                &id,
                ServerValidationError::BungeecordChannelNotIntercepted {
                    proxy_mode: config.proxy_mode,
                },
            ));
        }
    }

    validate_server_forwarding(config, ForwardingMode::None)?;

    if config.addresses.is_empty() {
        return Err(ConfigError::NoAddresses { id });
    }

    if config.domains.iter().any(|domain| domain.trim().is_empty()) {
        return Err(server_error(&id, ServerValidationError::EmptyDomain));
    }

    if let Some(name) = &config.name {
        validate_identifier(name, "name", &id)?;
    }

    if let Some(network) = &config.network {
        validate_identifier(network, "network", &id)?;
    }

    if !config.slow_start_aggression.is_finite() || config.slow_start_aggression <= 0.0 {
        return Err(server_error(
            &id,
            ServerValidationError::SlowStartAggression {
                value: config.slow_start_aggression,
            },
        ));
    }

    #[allow(unused_mut)]
    let mut warnings = balance_warnings(config);

    #[cfg(not(target_os = "linux"))]
    if config.proxy_mode == crate::types::ProxyMode::ZeroCopy {
        warnings.push("proxy_mode = zero_copy is only supported on Linux".to_string());
    }

    Ok(warnings)
}

pub fn validate_server_forwarding(
    config: &ServerConfig,
    default_mode: ForwardingMode,
) -> Result<(), ConfigError> {
    let mode = config.forwarding_mode.as_ref().unwrap_or(&default_mode);
    if config.proxy_mode.is_forwarding() && *mode == ForwardingMode::Velocity {
        let source = if config.forwarding_mode.is_some() {
            "its forwarding_mode"
        } else {
            "the proxy-wide [forwarding] mode"
        };
        return Err(server_error(
            &config.effective_id(),
            ServerValidationError::VelocityOnForwardingServer {
                proxy_mode: config.proxy_mode,
                mode_source: source,
            },
        ));
    }
    Ok(())
}

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

pub(crate) const MAX_IDENTIFIER_LEN: usize = 64;

fn validate_identifier(
    value: &str,
    field: &'static str,
    server_id: &str,
) -> Result<(), ConfigError> {
    validate_ident_chars(value, field, server_id, false)
}

fn validate_effective_id(id: &str) -> Result<(), ConfigError> {
    validate_ident_chars(id, "id", id, true)
}

fn validate_ident_chars(
    value: &str,
    field: &'static str,
    server_id: &str,
    allow_dot: bool,
) -> Result<(), ConfigError> {
    if value.is_empty() {
        return Err(server_error(
            server_id,
            ServerValidationError::EmptyIdentifier { field },
        ));
    }
    if value.len() > MAX_IDENTIFIER_LEN {
        return Err(server_error(
            server_id,
            ServerValidationError::IdentifierTooLong { field },
        ));
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
        return Err(server_error(
            server_id,
            ServerValidationError::IdentifierChars {
                field,
                value: value.to_string(),
                allowed,
            },
        ));
    }
    Ok(())
}

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

pub fn validate_proxy_config(config: &ProxyConfig) -> Result<Vec<String>, ConfigError> {
    if !config.servers_dir.is_dir() {
        return Err(ConfigError::DirectoryNotFound(config.servers_dir.clone()));
    }

    let mut warnings = validate_proxy_document(config)?;

    if config
        .forwarding
        .as_ref()
        .is_some_and(ForwardingConfig::has_moved_channel_keys)
    {
        warnings.push(
            "[forwarding] bungeecord_channel and [forwarding.channel_permissions] are no longer \
             read: use [plugin_messaging] bungeecord, [plugin_messaging.bungeecord_permissions] \
             and bungeecord_channel = true in the server files"
                .to_string(),
        );
    }

    if !config.plugins.is_empty() && !config.plugins_dir.is_dir() {
        warnings.push(format!(
            "plugins are configured but plugins_dir {} does not exist \
             (only built-in plugins will be available)",
            config.plugins_dir.display()
        ));
    }

    Ok(warnings)
}

fn zero_duration(key: &'static str) -> ConfigError {
    ConfigError::Proxy(ProxyValidationError::ZeroDuration { key })
}

pub fn validate_proxy_document(config: &ProxyConfig) -> Result<Vec<String>, ConfigError> {
    for (key, value) in [
        ("connect_timeout", config.connect_timeout),
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
        (
            "events.transport_filter_timeout",
            config.events.transport_filter_timeout,
        ),
        ("ban.check_timeout", config.ban.check_timeout),
    ] {
        if value.is_zero() {
            return Err(zero_duration(key));
        }
    }

    let warnings = validate_wasm_config(config)?;

    if config.rate_limit.enabled {
        if config.rate_limit.window.is_zero() {
            return Err(zero_duration("rate_limit.window"));
        }
        if config.rate_limit.status_window.is_zero() {
            return Err(zero_duration("rate_limit.status_window"));
        }
    }

    if let Some(docker) = &config.docker
        && docker.poll_interval.is_zero()
    {
        return Err(zero_duration("docker.poll_interval"));
    }

    if let Some(web) = &config.web {
        validate_web_bind(&web.bind, config.bind).map_err(ConfigError::Proxy)?;
        if web.enable_webui == Some(true) && !web.enable_api {
            return Err(ConfigError::Proxy(ProxyValidationError::WebUiWithoutApi));
        }
        if web.enable_api {
            web.check_api_key()
                .map_err(|reason| ConfigError::Proxy(ProxyValidationError::WebApiKey(reason)))?;
        }
    }

    Ok(warnings)
}

fn validate_web_bind(bind: &str, proxy_bind: SocketAddr) -> Result<(), ProxyValidationError> {
    let (web_ip, port) = if let Ok(addr) = bind.parse::<SocketAddr>() {
        (Some(addr.ip()), addr.port())
    } else {
        let Some((host, port_str)) = bind.rsplit_once(':') else {
            return Err(ProxyValidationError::WebBindNotHostPort {
                bind: bind.to_string(),
            });
        };
        let Ok(port) = port_str.parse::<u16>() else {
            return Err(ProxyValidationError::WebBindInvalidPort {
                bind: bind.to_string(),
                port: port_str.to_string(),
            });
        };
        let host = host.trim_matches(['[', ']']);
        if host.is_empty() || host.chars().any(char::is_whitespace) {
            return Err(ProxyValidationError::WebBindInvalidHost {
                bind: bind.to_string(),
            });
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
            return Err(ProxyValidationError::WebBindCollision {
                bind: bind.to_string(),
                proxy_bind,
            });
        }
    }

    Ok(())
}

pub(crate) const WASM_MIN_EPOCH_TICK: Duration = Duration::from_millis(1);
pub(crate) const WASM_MAX_EPOCH_TICK: Duration = Duration::from_secs(1);
pub(crate) const WASM_MAX_DURATION: Duration = Duration::from_secs(3600);
pub(crate) const WASM_MAX_MEMORY_MB: u32 = 4096;
pub(crate) const WASM_MAX_QUEUE_CAPACITY: usize = 1 << 20;
pub(crate) const WASM_MAX_INSTANCE_POOL: u32 = 32_768;
pub(crate) const WASM_MAX_RESTARTS: u32 = 1000;
pub(crate) const WASM_MAX_QUOTA: usize = 1 << 20;
const WASM_MAX_RECOVERY_DURATION: Duration = Duration::from_secs(86_400);

pub fn validate_wasm_config(config: &ProxyConfig) -> Result<Vec<String>, ConfigError> {
    validate_wasm(config).map_err(ConfigError::Wasm)?;
    Ok(wasm_warnings(config))
}

fn validate_wasm(config: &ProxyConfig) -> Result<(), WasmValidationError> {
    let tick = config.wasm.epoch_tick;
    if !(WASM_MIN_EPOCH_TICK..=WASM_MAX_EPOCH_TICK).contains(&tick) {
        return Err(WasmValidationError::EpochTickOutOfRange { tick });
    }
    if config.wasm.instance_pool > WASM_MAX_INSTANCE_POOL {
        return Err(WasmValidationError::InstancePoolTooLarge {
            value: config.wasm.instance_pool,
        });
    }
    validate_wasm_cache_dir(&config.wasm.cache_dir, &config.plugins_dir)?;
    for (scope, plugin, limits) in wasm_scopes(config) {
        validate_wasm_limits(&scope, &limits, tick)?;
        if let Some(plugin) = plugin {
            validate_wasm_mounts(&format!("{scope}.mounts"), plugin)?;
        }
    }
    Ok(())
}

fn validate_wasm_cache_dir(
    cache_dir: &Path,
    plugins_dir: &Path,
) -> Result<(), WasmValidationError> {
    if cache_dir.as_os_str().is_empty() {
        return Err(WasmValidationError::CacheDirEmpty);
    }
    if lexically_within(cache_dir, plugins_dir) {
        return Err(WasmValidationError::CacheDirInPluginsDir {
            cache_dir: cache_dir.to_path_buf(),
            plugins_dir: plugins_dir.to_path_buf(),
        });
    }
    Ok(())
}

fn lexically_within(inner: &Path, outer: &Path) -> bool {
    match (std::path::absolute(inner), std::path::absolute(outer)) {
        (Ok(inner), Ok(outer)) => normalized(&inner).starts_with(normalized(&outer)),
        _ => false,
    }
}

fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn wasm_scopes(
    config: &ProxyConfig,
) -> impl Iterator<Item = (String, Option<&PluginWasmConfig>, WasmLimits)> {
    let mut ids: Vec<&String> = config.plugins.keys().collect();
    ids.sort();
    std::iter::once(("wasm".to_string(), None, config.wasm.limits())).chain(
        ids.into_iter().filter_map(move |id| {
            let plugin = config.plugins[id].wasm.as_ref()?;
            Some((
                format!("plugins.{id}.wasm"),
                Some(plugin),
                config.wasm.limits_for(Some(plugin)),
            ))
        }),
    )
}

pub fn wasm_warnings(config: &ProxyConfig) -> Vec<String> {
    wasm_scopes(config)
        .filter(|(_, _, limits)| limits.cpu_budget > limits.max_call_duration)
        .map(|(scope, _, limits)| {
            format!(
                "{scope}: cpu_budget ({}) is longer than max_call_duration ({}); \
                 max_call_duration stops a busy guest call first",
                humantime::format_duration(limits.cpu_budget),
                humantime::format_duration(limits.max_call_duration)
            )
        })
        .collect()
}

fn validate_wasm_limits(
    scope: &str,
    limits: &WasmLimits,
    tick: Duration,
) -> Result<(), WasmValidationError> {
    if !(1..=WASM_MAX_MEMORY_MB).contains(&limits.memory_limit_mb) {
        return Err(WasmValidationError::MemoryLimitOutOfRange {
            scope: scope.to_string(),
            value: limits.memory_limit_mb,
        });
    }
    for (key, value) in [
        ("cpu_budget", limits.cpu_budget),
        ("codec_cpu_budget", limits.codec_cpu_budget),
    ] {
        if value < tick || value > WASM_MAX_DURATION {
            return Err(WasmValidationError::CpuBudgetOutOfRange {
                scope: scope.to_string(),
                key,
                tick,
                value,
            });
        }
    }
    for (key, value) in [
        ("host_call_timeout", limits.host_call_timeout),
        ("max_call_duration", limits.max_call_duration),
    ] {
        if value.is_zero() || value > WASM_MAX_DURATION {
            return Err(WasmValidationError::DurationOutOfRange {
                scope: scope.to_string(),
                key,
                max: WASM_MAX_DURATION,
                value,
            });
        }
    }
    if !(1..=WASM_MAX_QUEUE_CAPACITY).contains(&limits.queue_capacity) {
        return Err(WasmValidationError::QueueCapacityOutOfRange {
            scope: scope.to_string(),
            value: limits.queue_capacity,
        });
    }
    validate_wasm_recovery(scope, &limits.recovery)?;
    validate_wasm_quotas(scope, &limits.quotas)
}

fn validate_wasm_quotas(scope: &str, quotas: &WasmQuotasConfig) -> Result<(), WasmValidationError> {
    for (key, value) in quotas.entries() {
        if !(1..=WASM_MAX_QUOTA).contains(&value) {
            return Err(WasmValidationError::QuotaOutOfRange {
                scope: scope.to_string(),
                key,
                value,
            });
        }
    }
    Ok(())
}

fn validate_wasm_mounts(scope: &str, plugin: &PluginWasmConfig) -> Result<(), WasmValidationError> {
    let mut guests: Vec<String> = Vec::with_capacity(plugin.mounts.len());
    for mount in &plugin.mounts {
        if mount.host.as_os_str().is_empty() {
            return Err(WasmValidationError::MountHostEmpty {
                scope: scope.to_string(),
                guest: mount.guest.clone(),
            });
        }
        let guest = mount
            .guest_path()
            .map_err(|reason| WasmValidationError::MountGuestPath {
                scope: scope.to_string(),
                reason,
            })?;
        for other in &guests {
            let (short, long) = if other.len() <= guest.len() {
                (other.as_str(), guest.as_str())
            } else {
                (guest.as_str(), other.as_str())
            };
            if short == long {
                return Err(WasmValidationError::MountDuplicate {
                    scope: scope.to_string(),
                    guest,
                });
            }
            if long.starts_with(short) && long.as_bytes()[short.len()] == b'/' {
                return Err(WasmValidationError::MountOverlap {
                    scope: scope.to_string(),
                    short: short.to_string(),
                    long: long.to_string(),
                });
            }
        }
        guests.push(guest);
    }
    Ok(())
}

fn validate_wasm_recovery(
    scope: &str,
    recovery: &WasmRecoveryConfig,
) -> Result<(), WasmValidationError> {
    if recovery.max_restarts > WASM_MAX_RESTARTS {
        return Err(WasmValidationError::MaxRestartsTooLarge {
            scope: scope.to_string(),
            value: recovery.max_restarts,
        });
    }
    for (key, value) in [
        ("recovery.window", recovery.window),
        ("recovery.backoff_initial", recovery.backoff_initial),
        ("recovery.backoff_max", recovery.backoff_max),
    ] {
        if value.is_zero() || value > WASM_MAX_RECOVERY_DURATION {
            return Err(WasmValidationError::DurationOutOfRange {
                scope: scope.to_string(),
                key,
                max: WASM_MAX_RECOVERY_DURATION,
                value,
            });
        }
    }
    if recovery.backoff_initial > recovery.backoff_max {
        return Err(WasmValidationError::BackoffInitialExceedsMax {
            scope: scope.to_string(),
            initial: recovery.backoff_initial,
            max: recovery.backoff_max,
        });
    }
    Ok(())
}
