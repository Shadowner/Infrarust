//! V1 → V2 server config conversion logic.

use std::time::Duration;

use crate::server::ServerConfig;
use crate::types::{
    DomainRewrite, IpFilterConfig, LocalManagerConfig, MotdConfig, MotdEntry, ProxyMode,
    ServerAddress, ServerManagerConfig,
};

use super::v1_types::{V1MotdEntry, V1ServerConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug)]
pub struct MigrationWarning {
    pub severity: MigrationSeverity,
    pub file: String,
    pub message: String,
}

pub struct MigrationResult {
    pub config: ServerConfig,
    pub warnings: Vec<MigrationWarning>,
}

pub fn convert_v1_to_v2(v1: &V1ServerConfig, filename: &str) -> MigrationResult {
    let mut warnings = Vec::new();
    let warn = |warnings: &mut Vec<MigrationWarning>, severity: MigrationSeverity, msg: &str| {
        warnings.push(MigrationWarning {
            severity,
            file: filename.to_string(),
            message: msg.to_string(),
        });
    };

    let name = v1
        .config_id
        .as_ref()
        .map(|id| {
            id.split('@')
                .next()
                .unwrap_or(id)
                .to_lowercase()
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
        })
        .or_else(|| {
            let stem = filename
                .strip_suffix(".yaml")
                .or_else(|| filename.strip_suffix(".yml"))
                .unwrap_or(filename);
            Some(
                stem.to_lowercase()
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect(),
            )
        });

    let proxy_mode = match v1.proxy_mode.as_deref() {
        Some("passthrough") | None => ProxyMode::Passthrough,
        Some("zerocopy_passthrough") => ProxyMode::ZeroCopy,
        Some("offline") => ProxyMode::Offline,
        Some("client_only") => ProxyMode::ClientOnly,
        Some("server_only") => ProxyMode::ServerOnly,
        Some(unknown) => {
            warn(
                &mut warnings,
                MigrationSeverity::Warning,
                &format!("Unknown proxy mode '{unknown}', defaulting to passthrough"),
            );
            ProxyMode::Passthrough
        }
    };

    let domain_rewrite = if let Some(ref domain) = v1.backend_domain {
        DomainRewrite::Explicit(domain.clone())
    } else if v1.rewrite_domain == Some(true) {
        DomainRewrite::FromBackend
    } else {
        DomainRewrite::None
    };

    if v1.proxy_protocol_version.is_some() {
        warn(
            &mut warnings,
            MigrationSeverity::Info,
            "proxy_protocol_version is no longer supported in V2 (auto-detected)",
        );
    }

    let mut addresses: Vec<ServerAddress> = Vec::new();
    for addr in &v1.addresses {
        match addr.parse() {
            Ok(parsed) => addresses.push(parsed),
            Err(e) => warn(
                &mut warnings,
                MigrationSeverity::Error,
                &format!("Cannot parse address '{addr}': {e}"),
            ),
        }
    }
    if addresses.is_empty() && !v1.addresses.is_empty() {
        warn(
            &mut warnings,
            MigrationSeverity::Error,
            "ALL addresses failed to parse — this server will have no backends!",
        );
    }

    let motd = convert_motds(v1, filename, &mut warnings);
    let server_manager = convert_server_manager(v1, filename, &mut warnings);
    let ip_filter = convert_ip_filter(v1, filename, &mut warnings);

    if let Some(ref filters) = v1.filters {
        if filters.rate_limiter.as_ref().is_some_and(|f| f.enabled) {
            warn(
                &mut warnings,
                MigrationSeverity::Info,
                "Per-server rate limiter is now global in V2 (see infrarust.toml [rate_limit])",
            );
        }
        if filters.id_filter.as_ref().is_some_and(|f| f.enabled) {
            warn(
                &mut warnings,
                MigrationSeverity::Warning,
                "ID filter is not available in V2 server config",
            );
        }
        if filters.name_filter.as_ref().is_some_and(|f| f.enabled) {
            warn(
                &mut warnings,
                MigrationSeverity::Warning,
                "Name filter is not available in V2 server config",
            );
        }
        if filters.ban.as_ref().is_some_and(|f| f.enabled) {
            warn(
                &mut warnings,
                MigrationSeverity::Info,
                "Per-server ban config is now global in V2 (see infrarust.toml [ban])",
            );
        }
    }

    if v1.caches.is_some() {
        warn(
            &mut warnings,
            MigrationSeverity::Info,
            "Per-server cache config is now global in V2 (see infrarust.toml [status_cache])",
        );
    }

    let max_players = v1
        .motds
        .as_ref()
        .and_then(|m| m.online.as_ref())
        .and_then(|e| e.max_players)
        .unwrap_or(0);

    let config = ServerConfig {
        id: None,
        name,
        network: None,
        domains: v1.domains.clone(),
        addresses,
        proxy_mode,
        forwarding_mode: None,
        send_proxy_protocol: v1.send_proxy_protocol.unwrap_or(false),
        domain_rewrite,
        motd,
        server_manager,
        timeouts: None,
        max_players,
        ip_filter,
        disconnect_message: None,
        limbo_handlers: Vec::new(),
    };

    MigrationResult { config, warnings }
}

fn convert_v1_motd_entry(entry: &V1MotdEntry) -> Option<MotdEntry> {
    if !entry.enabled {
        return None;
    }
    let text = entry.text.clone()?;
    Some(MotdEntry {
        text,
        favicon: entry.favicon.clone(),
        version_name: entry.version_name.clone(),
        max_players: entry.max_players,
    })
}

fn convert_motds(
    v1: &V1ServerConfig,
    filename: &str,
    warnings: &mut Vec<MigrationWarning>,
) -> MotdConfig {
    let motds = match &v1.motds {
        Some(m) => m,
        None => return MotdConfig::default(),
    };

    let online = motds.online.as_ref().and_then(convert_v1_motd_entry);
    let starting = motds.starting.as_ref().and_then(convert_v1_motd_entry);
    let crashed = motds.crashed.as_ref().and_then(convert_v1_motd_entry);

    let sleeping = motds.offline.as_ref().and_then(convert_v1_motd_entry);

    let v1_stopping = motds.stopping.as_ref().and_then(convert_v1_motd_entry);
    let v1_shutting_down = motds.shutting_down.as_ref().and_then(convert_v1_motd_entry);
    let stopping = match (&v1_stopping, &v1_shutting_down) {
        (Some(_), Some(_)) => {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Info,
                file: filename.to_string(),
                message: "Both 'stopping' and 'shutting_down' MOTDs exist; using 'shutting_down'"
                    .to_string(),
            });
            v1_shutting_down
        }
        (_, Some(_)) => v1_shutting_down,
        (Some(_), None) => v1_stopping,
        (None, None) => None,
    };

    let v1_unreachable = motds.unreachable.as_ref().and_then(convert_v1_motd_entry);
    let has_unknown = motds.unknown.as_ref().is_some_and(|e| e.enabled);
    let has_unable = motds.unable_status.as_ref().is_some_and(|e| e.enabled);
    let v1_unknown = motds.unknown.as_ref().and_then(convert_v1_motd_entry);
    let v1_unable = motds.unable_status.as_ref().and_then(convert_v1_motd_entry);
    let unreachable = v1_unreachable.or(v1_unknown).or(v1_unable);

    if has_unknown || has_unable {
        let has_unreachable = motds.unreachable.as_ref().is_some_and(|e| e.enabled);
        let msg = if has_unreachable {
            "'unknown'/'unable_status' MOTDs merged into 'unreachable' (using 'unreachable')"
        } else {
            "'unknown'/'unable_status' MOTDs renamed to 'unreachable' in V2"
        };
        warnings.push(MigrationWarning {
            severity: MigrationSeverity::Info,
            file: filename.to_string(),
            message: msg.to_string(),
        });
    }

    let has_dropped_fields = |entry: &Option<V1MotdEntry>| -> bool {
        entry.as_ref().is_some_and(|e| {
            e.enabled
                && (e.protocol_version.is_some()
                    || e.online_players.is_some()
                    || !e.samples.is_empty())
        })
    };

    let all_entries = [
        &motds.online,
        &motds.offline,
        &motds.unreachable,
        &motds.starting,
        &motds.stopping,
        &motds.shutting_down,
        &motds.crashed,
        &motds.unknown,
        &motds.unable_status,
    ];

    if all_entries.iter().any(|e| has_dropped_fields(e)) {
        warnings.push(MigrationWarning {
            severity: MigrationSeverity::Info,
            file: filename.to_string(),
            message: "MOTD fields 'protocol_version', 'online_players', 'samples' are not supported in V2 and were dropped".to_string(),
        });
    }

    MotdConfig {
        online,
        offline: None,
        sleeping,
        starting,
        crashed,
        stopping,
        unreachable,
    }
}

fn convert_server_manager(
    v1: &V1ServerConfig,
    filename: &str,
    warnings: &mut Vec<MigrationWarning>,
) -> Option<ServerManagerConfig> {
    let sm = v1.server_manager.as_ref()?;
    let provider = match sm.provider_name.as_deref() {
        Some(p) => p,
        None => {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: filename.to_string(),
                message: "server_manager has no provider_name, skipping".to_string(),
            });
            return None;
        }
    };

    let shutdown_after = sm.empty_shutdown_time.map(Duration::from_secs);

    match provider.to_lowercase().as_str() {
        "local" => {
            let lp = match sm.local_provider.as_ref() {
                Some(lp) => lp,
                None => {
                    warnings.push(MigrationWarning {
                        severity: MigrationSeverity::Error,
                        file: filename.to_string(),
                        message: "Local server manager has no local_provider config, skipping"
                            .to_string(),
                    });
                    return None;
                }
            };
            Some(ServerManagerConfig::Local(LocalManagerConfig {
                command: lp.executable.clone().unwrap_or_else(|| "java".to_string()),
                working_dir: lp
                    .working_dir
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from(".")),
                args: lp.args.clone(),
                ready_pattern: lp
                    .startup_string
                    .clone()
                    .unwrap_or_else(crate::defaults::ready_pattern),
                shutdown_timeout: crate::defaults::shutdown_timeout(),
                shutdown_after,
                start_timeout: crate::defaults::start_timeout(),
            }))
        }
        "pterodactyl" => {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: filename.to_string(),
                message: "Pterodactyl api_url and api_key must be filled in manually (stored globally in V1)".to_string(),
            });
            let server_id = sm.server_id.clone().unwrap_or_else(|| {
                warnings.push(MigrationWarning {
                    severity: MigrationSeverity::Error,
                    file: filename.to_string(),
                    message: "Pterodactyl server_id is missing, must be filled in manually"
                        .to_string(),
                });
                "TODO: fill in your server ID".to_string()
            });
            Some(ServerManagerConfig::Pterodactyl(
                crate::types::PterodactylManagerConfig {
                    api_url: "TODO: fill in your Pterodactyl panel URL".to_string(),
                    api_key: "TODO: fill in your API key".to_string(),
                    server_id,
                    shutdown_after,
                    start_timeout: crate::defaults::start_timeout(),
                    poll_interval: crate::defaults::poll_interval(),
                },
            ))
        }
        "crafty" => {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: filename.to_string(),
                message:
                    "Crafty api_url and api_key must be filled in manually (stored globally in V1)"
                        .to_string(),
            });
            let server_id = sm.server_id.clone().unwrap_or_else(|| {
                warnings.push(MigrationWarning {
                    severity: MigrationSeverity::Error,
                    file: filename.to_string(),
                    message: "Crafty server_id is missing, must be filled in manually".to_string(),
                });
                "TODO: fill in your server ID".to_string()
            });
            Some(ServerManagerConfig::Crafty(
                crate::types::CraftyManagerConfig {
                    api_url: "TODO: fill in your Crafty Controller URL".to_string(),
                    api_key: "TODO: fill in your API key".to_string(),
                    server_id,
                    shutdown_after,
                    start_timeout: crate::defaults::start_timeout(),
                    poll_interval: crate::defaults::poll_interval(),
                },
            ))
        }
        other => {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: filename.to_string(),
                message: format!(
                    "Server manager provider '{other}' is not supported in V2, skipping"
                ),
            });
            None
        }
    }
}

fn convert_ip_filter(
    v1: &V1ServerConfig,
    filename: &str,
    warnings: &mut Vec<MigrationWarning>,
) -> Option<IpFilterConfig> {
    let filters = v1.filters.as_ref()?;
    let ip = filters.ip_filter.as_ref()?;

    if !ip.enabled {
        return None;
    }

    let mut parse_ip_list = |list: &[String]| -> Vec<ipnet::IpNet> {
        list.iter()
            .filter_map(|s| {
                if let Ok(net) = s.parse::<ipnet::IpNet>() {
                    return Some(net);
                }
                if let Ok(ip) = s.parse::<std::net::IpAddr>() {
                    return Some(ipnet::IpNet::from(ip));
                }
                warnings.push(MigrationWarning {
                    severity: MigrationSeverity::Warning,
                    file: filename.to_string(),
                    message: format!("Cannot parse IP filter entry '{s}', skipping"),
                });
                None
            })
            .collect()
    };

    let whitelist = parse_ip_list(&ip.whitelist);
    let blacklist = parse_ip_list(&ip.blacklist);

    if whitelist.is_empty() && blacklist.is_empty() {
        if !ip.whitelist.is_empty() || !ip.blacklist.is_empty() {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: filename.to_string(),
                message: "All IP filter entries failed to parse; IP filter removed entirely"
                    .to_string(),
            });
        }
        return None;
    }

    Some(IpFilterConfig {
        whitelist,
        blacklist,
    })
}

use super::v1_types::V1InfrarustConfig;
use crate::proxy::ProxyConfig;
use crate::types::{
    BanConfig, DockerProviderConfig, KeepaliveConfig, MetricsConfig, RateLimitConfig,
    ResourceConfig, StatusCacheConfig, TelemetryConfig, TracesConfig,
};

pub struct ProxyMigrationResult {
    pub config: ProxyConfig,
    pub warnings: Vec<MigrationWarning>,
}

pub fn convert_v1_proxy_config(v1: &V1InfrarustConfig) -> ProxyMigrationResult {
    let mut warnings = Vec::new();
    let file = "config.yaml".to_string();

    let bind = v1
        .bind
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(crate::defaults::bind);

    let keepalive = v1
        .keep_alive_timeout
        .as_ref()
        .and_then(|s| humantime::parse_duration(s).ok())
        .map(|time| KeepaliveConfig {
            time,
            ..KeepaliveConfig::default()
        })
        .unwrap_or_default();

    let servers_dir = v1
        .file_provider
        .as_ref()
        .and_then(|fp| fp.proxies_path.first())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::defaults::servers_dir);

    if let Some(fp) = &v1.file_provider
        && fp.proxies_path.len() > 1
    {
        warnings.push(MigrationWarning {
            severity: MigrationSeverity::Warning,
            file: file.clone(),
            message: format!(
                "V2 supports only one servers_dir; using '{}', ignoring {} other path(s)",
                servers_dir.display(),
                fp.proxies_path.len() - 1
            ),
        });
    }

    if v1.handshake_timeout_secs.is_some() || v1.status_request_timeout_secs.is_some() {
        warnings.push(MigrationWarning {
            severity: MigrationSeverity::Info,
            file: file.clone(),
            message: "handshake_timeout_secs and status_request_timeout_secs are not in V2"
                .to_string(),
        });
    }

    let rate_limit = v1
        .filters
        .as_ref()
        .and_then(|f| f.rate_limiter.as_ref())
        .map(|rl| {
            let window = rl
                .window_length
                .as_ref()
                .and_then(|s| humantime::parse_duration(s).ok())
                .unwrap_or_else(crate::defaults::rate_limit_window);
            RateLimitConfig {
                max_connections: rl
                    .request_limit
                    .unwrap_or_else(crate::defaults::rate_limit_max),
                window,
                ..RateLimitConfig::default()
            }
        })
        .unwrap_or_default();

    if let Some(ref filters) = v1.filters {
        if filters.ip_filter.as_ref().is_some_and(|f| f.enabled) {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: file.clone(),
                message:
                    "Global IP filter is per-server in V2, must be configured in each server .toml"
                        .to_string(),
            });
        }
        if filters.id_filter.as_ref().is_some_and(|f| f.enabled) {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: file.clone(),
                message: "ID filter is not available in V2".to_string(),
            });
        }
        if filters.name_filter.as_ref().is_some_and(|f| f.enabled) {
            warnings.push(MigrationWarning {
                severity: MigrationSeverity::Warning,
                file: file.clone(),
                message: "Name filter is not available in V2".to_string(),
            });
        }
    }

    let ban = v1
        .filters
        .as_ref()
        .and_then(|f| f.ban.as_ref())
        .map(|b| BanConfig {
            file: b
                .file_path
                .as_ref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(crate::defaults::ban_file),
            purge_interval: b
                .auto_cleanup_interval
                .map(Duration::from_secs)
                .unwrap_or_else(crate::defaults::ban_purge_interval),
            enable_audit_log: b
                .enable_audit_log
                .unwrap_or_else(crate::defaults::ban_audit_log),
        })
        .unwrap_or_default();

    let status_cache = v1
        .cache
        .as_ref()
        .map(|c| StatusCacheConfig {
            ttl: c
                .status_ttl_seconds
                .map(Duration::from_secs)
                .unwrap_or_else(crate::defaults::status_cache_ttl),
            max_entries: c
                .max_status_entries
                .unwrap_or_else(crate::defaults::status_cache_max_entries),
        })
        .unwrap_or_default();

    let telemetry = v1.telemetry.as_ref().map(|t| TelemetryConfig {
        enabled: t.enabled,
        endpoint: t.export_url.clone(),
        protocol: crate::defaults::telemetry_protocol(),
        metrics: MetricsConfig {
            enabled: t.enable_metrics.unwrap_or(true),
            export_interval: t
                .export_interval_seconds
                .map(Duration::from_secs)
                .unwrap_or_else(crate::defaults::metrics_export_interval),
        },
        traces: TracesConfig {
            enabled: t.enable_tracing.unwrap_or(true),
            ..TracesConfig::default()
        },
        resource: ResourceConfig::default(),
    });

    let receive_proxy_protocol = v1
        .proxy_protocol
        .as_ref()
        .and_then(|pp| pp.receive_enabled)
        .unwrap_or(false);

    if let Some(pp) = &v1.proxy_protocol
        && (pp.receive_timeout_secs.is_some() || pp.receive_allowed_versions.is_some())
    {
        warnings.push(MigrationWarning {
            severity: MigrationSeverity::Info,
            file: file.clone(),
            message:
                "proxy_protocol.receive_timeout_secs and receive_allowed_versions are not in V2"
                    .to_string(),
        });
    }

    let default_motd = v1.motds.as_ref().and_then(|motds| {
        let entry = motds
            .unreachable
            .as_ref()
            .or(motds.unknown.as_ref())
            .or(motds.unable_status.as_ref());
        entry.and_then(|e| {
            let text = e.text.clone()?;
            Some(MotdConfig {
                online: Some(MotdEntry {
                    text,
                    favicon: e.favicon.clone(),
                    version_name: e.version_name.clone(),
                    max_players: e.max_players,
                }),
                ..MotdConfig::default()
            })
        })
    });

    if v1.logging.is_some() {
        warnings.push(MigrationWarning {
            severity: MigrationSeverity::Info,
            file: file.clone(),
            message: "Logging config is not in V2; use RUST_LOG env var or --log-level CLI flag"
                .to_string(),
        });
    }

    if v1.managers_config.is_some() {
        warnings.push(MigrationWarning {
            severity: MigrationSeverity::Warning,
            file: file.clone(),
            message: "managers_config (Pterodactyl/Crafty credentials) is now per-server in V2 [server_manager] sections".to_string(),
        });
    }

    let docker = v1.docker_provider.as_ref().map(|dp| DockerProviderConfig {
        endpoint: dp
            .docker_host
            .clone()
            .unwrap_or_else(crate::defaults::docker_endpoint),
        network: None,
        poll_interval: dp
            .polling_interval
            .map(Duration::from_secs)
            .unwrap_or_else(crate::defaults::docker_poll_interval),
        reconnect_delay: crate::defaults::docker_reconnect_delay(),
    });

    let config = ProxyConfig {
        bind,
        max_connections: 0,
        connect_timeout: crate::defaults::connect_timeout(),
        receive_proxy_protocol,
        servers_dir,
        plugins_dir: crate::defaults::plugins_dir(),
        worker_threads: 0,
        rate_limit,
        status_cache,
        default_motd,
        telemetry,
        keepalive,
        so_reuseport: false,
        ban,
        docker,
        unknown_domain_behavior: Default::default(),
        announce_proxy_commands: crate::defaults::announce_proxy_commands(),
        forwarding: None,
        web: None,
        plugins: std::collections::HashMap::new(),
    };

    ProxyMigrationResult { config, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::v1_types::*;

    fn minimal_v1() -> V1ServerConfig {
        V1ServerConfig {
            domains: vec!["example.com".to_string()],
            addresses: vec!["127.0.0.1:25565".to_string()],
            proxy_mode: None,
            send_proxy_protocol: None,
            proxy_protocol_version: None,
            backend_domain: None,
            rewrite_domain: None,
            config_id: None,
            server_manager: None,
            motds: None,
            filters: None,
            caches: None,
        }
    }

    #[test]
    fn test_minimal_conversion() {
        let result = convert_v1_to_v2(&minimal_v1(), "test.yaml");
        assert_eq!(result.config.domains, vec!["example.com"]);
        assert_eq!(result.config.proxy_mode, ProxyMode::Passthrough);
        assert_eq!(result.config.name, Some("test".to_string()));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_proxy_mode_mapping() {
        let mut v1 = minimal_v1();
        v1.proxy_mode = Some("zerocopy_passthrough".to_string());
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert_eq!(result.config.proxy_mode, ProxyMode::ZeroCopy);
    }

    #[test]
    fn test_domain_rewrite_explicit() {
        let mut v1 = minimal_v1();
        v1.backend_domain = Some("backend.example.com".to_string());
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert_eq!(
            result.config.domain_rewrite,
            DomainRewrite::Explicit("backend.example.com".to_string())
        );
    }

    #[test]
    fn test_domain_rewrite_from_backend() {
        let mut v1 = minimal_v1();
        v1.rewrite_domain = Some(true);
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert_eq!(result.config.domain_rewrite, DomainRewrite::FromBackend);
    }

    #[test]
    fn test_config_id_sanitization() {
        let mut v1 = minimal_v1();
        v1.config_id = Some("My Server!@file_provider".to_string());
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert_eq!(result.config.name, Some("my-server-".to_string()));
    }

    #[test]
    fn test_motd_offline_becomes_sleeping() {
        let mut v1 = minimal_v1();
        v1.motds = Some(V1Motds {
            online: None,
            offline: Some(V1MotdEntry {
                enabled: true,
                text: Some("Sleeping".to_string()),
                version_name: None,
                max_players: None,
                online_players: None,
                protocol_version: None,
                favicon: None,
                samples: vec![],
            }),
            unreachable: None,
            starting: None,
            stopping: None,
            shutting_down: None,
            crashed: None,
            unknown: None,
            unable_status: None,
        });
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert!(result.config.motd.sleeping.is_some());
        assert_eq!(result.config.motd.sleeping.unwrap().text, "Sleeping");
    }

    #[test]
    fn test_disabled_motd_is_skipped() {
        let mut v1 = minimal_v1();
        v1.motds = Some(V1Motds {
            online: Some(V1MotdEntry {
                enabled: false,
                text: Some("Online".to_string()),
                version_name: None,
                max_players: None,
                online_players: None,
                protocol_version: None,
                favicon: None,
                samples: vec![],
            }),
            offline: None,
            unreachable: None,
            starting: None,
            stopping: None,
            shutting_down: None,
            crashed: None,
            unknown: None,
            unable_status: None,
        });
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert!(result.config.motd.online.is_none());
    }

    #[test]
    fn test_warns_on_dropped_fields() {
        let mut v1 = minimal_v1();
        v1.proxy_protocol_version = Some(2);
        v1.caches = Some(V1Caches {
            status_ttl_seconds: Some(30),
            max_status_entries: None,
        });
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert!(result.warnings.len() >= 2);
    }
}
