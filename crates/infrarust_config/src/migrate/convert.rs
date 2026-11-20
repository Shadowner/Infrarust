//! V1 → V2 server config conversion logic.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::proxy::ProxyConfig;
use crate::server::ServerConfig;
use crate::types::{
    ActiveHealthConfig, BalanceStrategy, BanConfig, CraftyManagerConfig, DockerProviderConfig,
    DomainRewrite, IpFilterConfig, KeepaliveConfig, LocalManagerConfig, MetricsConfig, MotdConfig,
    MotdEntry, OtlpProtocol, ProxyMode, PterodactylManagerConfig, RateLimitConfig, ResourceConfig,
    ServerAddress, ServerManagerConfig, StatusCacheConfig, TelemetryConfig, TracesConfig,
    WeightedAddress,
};

use super::v1_types::{V1InfrarustConfig, V1MotdEntry, V1ServerConfig};

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

pub(super) struct Warnings {
    file: String,
    items: Vec<MigrationWarning>,
}

impl Warnings {
    pub(super) fn new(file: &str) -> Self {
        Self {
            file: file.to_string(),
            items: Vec::new(),
        }
    }

    pub(super) fn info(&mut self, message: impl Into<String>) {
        self.push(MigrationSeverity::Info, message);
    }

    pub(super) fn warn(&mut self, message: impl Into<String>) {
        self.push(MigrationSeverity::Warning, message);
    }

    pub(super) fn error(&mut self, message: impl Into<String>) {
        self.push(MigrationSeverity::Error, message);
    }

    pub(super) fn into_vec(self) -> Vec<MigrationWarning> {
        self.items
    }

    fn push(&mut self, severity: MigrationSeverity, message: impl Into<String>) {
        self.items.push(MigrationWarning {
            severity,
            file: self.file.clone(),
            message: message.into(),
        });
    }
}

pub struct MigrationResult {
    pub config: ServerConfig,
    pub warnings: Vec<MigrationWarning>,
}

pub fn convert_v1_to_v2(v1: &V1ServerConfig, filename: &str) -> MigrationResult {
    let mut warnings = Warnings::new(filename);
    let config = convert_server(v1, filename, &mut warnings);
    MigrationResult {
        config,
        warnings: warnings.into_vec(),
    }
}

pub(super) fn convert_server(
    v1: &V1ServerConfig,
    filename: &str,
    warnings: &mut Warnings,
) -> ServerConfig {
    let name = convert_name(v1, filename, warnings);
    let proxy_mode = convert_proxy_mode(v1, warnings);

    let domain_rewrite = if let Some(ref domain) = v1.backend_domain {
        DomainRewrite::Explicit(domain.clone())
    } else if v1.rewrite_domain == Some(true) {
        DomainRewrite::FromBackend
    } else {
        DomainRewrite::None
    };

    if v1.proxy_protocol_version.is_some() {
        warnings.info("proxy_protocol_version is no longer supported in V2 (auto-detected)");
    }

    let addresses = convert_addresses(v1, warnings);
    let motd = convert_motds(v1, warnings);
    let server_manager = convert_server_manager(v1, warnings);
    let ip_filter = convert_ip_filter(v1, warnings);
    warn_moved_server_settings(v1, warnings);

    let max_players = v1
        .motds
        .as_ref()
        .and_then(|m| m.online.as_ref())
        .and_then(|e| e.max_players)
        .unwrap_or(0);

    ServerConfig {
        id: None,
        name,
        network: None,
        active_health: None,
        domains: v1.domains.clone(),
        addresses,
        balance: BalanceStrategy::default(),
        slow_start: None,
        slow_start_aggression: 1.0,
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
        bungeecord_channel: false,
    }
}

fn convert_name(v1: &V1ServerConfig, filename: &str, warnings: &mut Warnings) -> Option<String> {
    let raw_name = v1
        .config_id
        .as_deref()
        .map(|id| id.split('@').next().unwrap_or(id))
        .unwrap_or_else(|| {
            filename
                .strip_suffix(".yaml")
                .or_else(|| filename.strip_suffix(".yml"))
                .unwrap_or(filename)
        });
    let sanitized = sanitize_id(raw_name);
    if sanitized.is_empty() {
        warnings.warn(format!(
            "config id '{raw_name}' has no usable characters; the V2 id will derive from the output filename"
        ));
        return None;
    }
    if sanitized != raw_name {
        warnings.warn(format!(
            "config id '{raw_name}' sanitized to '{sanitized}'; check for collisions with other migrated configs"
        ));
    }
    Some(sanitized)
}

fn convert_proxy_mode(v1: &V1ServerConfig, warnings: &mut Warnings) -> ProxyMode {
    match v1.proxy_mode.as_deref() {
        Some("passthrough") | None => ProxyMode::Passthrough,
        Some("zerocopy_passthrough") => ProxyMode::ZeroCopy,
        Some("offline") => ProxyMode::Offline,
        Some("client_only") => ProxyMode::ClientOnly,
        Some("server_only") => ProxyMode::ServerOnly,
        Some(unknown) => {
            warnings.warn(format!(
                "Unknown proxy mode '{unknown}', defaulting to passthrough"
            ));
            ProxyMode::Passthrough
        }
    }
}

fn convert_addresses(v1: &V1ServerConfig, warnings: &mut Warnings) -> Vec<WeightedAddress> {
    let mut addresses = Vec::new();
    for addr in &v1.addresses {
        match addr.parse::<ServerAddress>() {
            Ok(parsed) => addresses.push(parsed.into()),
            Err(e) => warnings.error(format!("Cannot parse address '{addr}': {e}")),
        }
    }
    if addresses.is_empty() && !v1.addresses.is_empty() {
        warnings.error("ALL addresses failed to parse — this server will have no backends!");
    }
    addresses
}

fn warn_moved_server_settings(v1: &V1ServerConfig, warnings: &mut Warnings) {
    if let Some(ref filters) = v1.filters {
        if filters.rate_limiter.as_ref().is_some_and(|f| f.enabled) {
            warnings.info(
                "Per-server rate limiter is now global in V2 (see infrarust.toml [rate_limit])",
            );
        }
        if filters.id_filter.as_ref().is_some_and(|f| f.enabled) {
            warnings.warn("ID filter is not available in V2 server config");
        }
        if filters.name_filter.as_ref().is_some_and(|f| f.enabled) {
            warnings.warn("Name filter is not available in V2 server config");
        }
        if filters.ban.as_ref().is_some_and(|f| f.enabled) {
            warnings.info("Per-server ban config is now global in V2 (see infrarust.toml [ban])");
        }
    }

    if v1.caches.is_some() {
        warnings.info(
            "Per-server cache config is now global in V2 (see infrarust.toml [status_cache])",
        );
    }
}

fn sanitize_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn convert_v1_motd_entry(entry: &V1MotdEntry) -> Option<MotdEntry> {
    if !entry.enabled {
        return None;
    }
    motd_entry(entry)
}

fn motd_entry(entry: &V1MotdEntry) -> Option<MotdEntry> {
    Some(MotdEntry {
        text: entry.text.clone()?,
        favicon: entry.favicon.clone(),
        version_name: entry.version_name.clone(),
        version_protocol: entry.protocol_version,
        max_players: entry.max_players,
    })
}

fn convert_motds(v1: &V1ServerConfig, warnings: &mut Warnings) -> MotdConfig {
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
    if v1_stopping.is_some() && v1_shutting_down.is_some() {
        warnings.info("Both 'stopping' and 'shutting_down' MOTDs exist; using 'shutting_down'");
    }
    let stopping = v1_shutting_down.or(v1_stopping);

    let v1_unreachable = motds.unreachable.as_ref().and_then(convert_v1_motd_entry);
    let has_unknown = motds.unknown.as_ref().is_some_and(|e| e.enabled);
    let has_unable = motds.unable_status.as_ref().is_some_and(|e| e.enabled);
    let v1_unknown = motds.unknown.as_ref().and_then(convert_v1_motd_entry);
    let v1_unable = motds.unable_status.as_ref().and_then(convert_v1_motd_entry);
    let unreachable = v1_unreachable.or(v1_unknown).or(v1_unable);

    if has_unknown || has_unable {
        let has_unreachable = motds.unreachable.as_ref().is_some_and(|e| e.enabled);
        warnings.info(if has_unreachable {
            "'unknown'/'unable_status' MOTDs merged into 'unreachable' (using 'unreachable')"
        } else {
            "'unknown'/'unable_status' MOTDs renamed to 'unreachable' in V2"
        });
    }

    let has_dropped_fields = |entry: &Option<V1MotdEntry>| -> bool {
        entry
            .as_ref()
            .is_some_and(|e| e.enabled && (e.online_players.is_some() || !e.samples.is_empty()))
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
        warnings.info(
            "MOTD fields 'online_players', 'samples' are not supported in V2 and were dropped",
        );
    }

    MotdConfig {
        online,
        sleeping,
        starting,
        crashed,
        stopping,
        unreachable,
        offline: None,
    }
}

fn convert_server_manager(
    v1: &V1ServerConfig,
    warnings: &mut Warnings,
) -> Option<ServerManagerConfig> {
    let sm = v1.server_manager.as_ref()?;
    let Some(provider) = sm.provider_name.as_deref() else {
        warnings.warn("server_manager has no provider_name, skipping");
        return None;
    };

    let shutdown_after = sm.empty_shutdown_time.map(Duration::from_secs);

    match provider.to_lowercase().as_str() {
        "local" => {
            let Some(lp) = sm.local_provider.as_ref() else {
                warnings.error("Local server manager has no local_provider config, skipping");
                return None;
            };
            Some(ServerManagerConfig::Local(LocalManagerConfig {
                command: lp.executable.clone().unwrap_or_else(|| "java".to_string()),
                working_dir: lp
                    .working_dir
                    .as_ref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(".")),
                args: lp.args.clone(),
                ready_pattern: lp
                    .startup_string
                    .clone()
                    .unwrap_or_else(crate::defaults::ready_pattern),
                shutdown_timeout: crate::defaults::shutdown_timeout(),
                shutdown_after,
                start_timeout: crate::defaults::start_timeout(),
                poll_interval: crate::defaults::poll_interval(),
            }))
        }
        "pterodactyl" => {
            warnings.warn(
                "Pterodactyl api_url and api_key must be filled in manually (stored globally in V1)",
            );
            let server_id = sm.server_id.clone().unwrap_or_else(|| {
                warnings.error("Pterodactyl server_id is missing, must be filled in manually");
                "TODO: fill in your server ID".to_string()
            });
            Some(ServerManagerConfig::Pterodactyl(PterodactylManagerConfig {
                api_url: "TODO: fill in your Pterodactyl panel URL".to_string(),
                api_key: "TODO: fill in your API key".to_string(),
                server_id,
                shutdown_after,
                start_timeout: crate::defaults::start_timeout(),
                poll_interval: crate::defaults::poll_interval(),
            }))
        }
        "crafty" => {
            warnings.warn(
                "Crafty api_url and api_key must be filled in manually (stored globally in V1)",
            );
            let server_id = sm.server_id.clone().unwrap_or_else(|| {
                warnings.error("Crafty server_id is missing, must be filled in manually");
                "TODO: fill in your server ID".to_string()
            });
            Some(ServerManagerConfig::Crafty(CraftyManagerConfig {
                api_url: "TODO: fill in your Crafty Controller URL".to_string(),
                api_key: "TODO: fill in your API key".to_string(),
                server_id,
                shutdown_after,
                start_timeout: crate::defaults::start_timeout(),
                poll_interval: crate::defaults::poll_interval(),
            }))
        }
        other => {
            warnings.warn(format!(
                "Server manager provider '{other}' is not supported in V2, skipping"
            ));
            None
        }
    }
}

fn convert_ip_filter(v1: &V1ServerConfig, warnings: &mut Warnings) -> Option<IpFilterConfig> {
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
                warnings.warn(format!("Cannot parse IP filter entry '{s}', skipping"));
                None
            })
            .collect()
    };

    let whitelist = parse_ip_list(&ip.whitelist);
    let blacklist = parse_ip_list(&ip.blacklist);

    if whitelist.is_empty() && blacklist.is_empty() {
        if !ip.whitelist.is_empty() || !ip.blacklist.is_empty() {
            warnings.warn("All IP filter entries failed to parse; IP filter removed entirely");
        }
        return None;
    }

    Some(IpFilterConfig {
        whitelist,
        blacklist,
    })
}

pub struct ProxyMigrationResult {
    pub config: ProxyConfig,
    pub warnings: Vec<MigrationWarning>,
}

pub fn convert_v1_proxy_config(v1: &V1InfrarustConfig) -> ProxyMigrationResult {
    let mut warnings = Warnings::new("config.yaml");

    let bind = convert_bind(v1, &mut warnings);
    let servers_dir = convert_servers_dir(v1, &mut warnings);

    if v1.handshake_timeout_secs.is_some() || v1.status_request_timeout_secs.is_some() {
        warnings.info("handshake_timeout_secs and status_request_timeout_secs are not in V2");
    }

    warn_dropped_global_filters(v1, &mut warnings);
    let receive_proxy_protocol = convert_receive_proxy_protocol(v1, &mut warnings);

    if v1.logging.is_some() {
        warnings.info("Logging config is not in V2; use RUST_LOG env var or --log-level CLI flag");
    }

    if v1.managers_config.is_some() {
        warnings.warn(
            "managers_config (Pterodactyl/Crafty credentials) is now per-server in V2 [server_manager] sections",
        );
    }

    let config = ProxyConfig {
        bind,
        active_health: ActiveHealthConfig::default(),
        events: Default::default(),
        plugin_messaging: Default::default(),
        wasm: Default::default(),
        max_connections: 0,
        max_connections_per_ip: crate::defaults::max_connections_per_ip(),
        connect_timeout: crate::defaults::connect_timeout(),
        connect_max_attempts: crate::defaults::connect_max_attempts(),
        receive_proxy_protocol,
        trusted_proxies: Vec::new(),
        servers_dir,
        plugins_dir: crate::defaults::plugins_dir(),
        worker_threads: 0,
        rate_limit: convert_rate_limit(v1),
        status_cache: convert_status_cache(v1),
        default_motd: convert_default_motd(v1),
        telemetry: convert_telemetry(v1),
        keepalive: convert_keepalive(v1),
        so_reuseport: false,
        ban: convert_ban(v1),
        docker: convert_docker(v1),
        unknown_domain_behavior: Default::default(),
        announce_proxy_commands: crate::defaults::true_val(),
        compression_threshold: crate::defaults::compression_threshold(),
        forwarding: None,
        auth: Default::default(),
        ip_filter: None,
        web: None,
        permissions: Default::default(),
        plugins: std::collections::HashMap::new(),
    };

    ProxyMigrationResult {
        config,
        warnings: warnings.into_vec(),
    }
}

fn convert_bind(v1: &V1InfrarustConfig, warnings: &mut Warnings) -> SocketAddr {
    let Some(raw) = v1.bind.as_deref() else {
        return crate::defaults::bind();
    };
    let infrared_style = || {
        raw.starts_with(':')
            .then(|| format!("0.0.0.0{raw}").parse().ok())?
    };
    raw.parse().ok().or_else(infrared_style).unwrap_or_else(|| {
        warnings.warn(format!(
            "Cannot parse bind '{raw}', falling back to {}",
            crate::defaults::bind()
        ));
        crate::defaults::bind()
    })
}

fn convert_servers_dir(v1: &V1InfrarustConfig, warnings: &mut Warnings) -> PathBuf {
    let paths = v1
        .file_provider
        .as_ref()
        .map(|fp| fp.proxies_path.as_slice())
        .unwrap_or_default();
    let servers_dir = paths
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(crate::defaults::servers_dir);

    if paths.len() > 1 {
        warnings.warn(format!(
            "V2 supports only one servers_dir; using '{}', ignoring {} other path(s)",
            servers_dir.display(),
            paths.len() - 1
        ));
    }
    servers_dir
}

fn convert_keepalive(v1: &V1InfrarustConfig) -> KeepaliveConfig {
    v1.keep_alive_timeout
        .as_ref()
        .and_then(|s| humantime::parse_duration(s).ok())
        .map(|time| KeepaliveConfig {
            time,
            ..KeepaliveConfig::default()
        })
        .unwrap_or_default()
}

fn convert_rate_limit(v1: &V1InfrarustConfig) -> RateLimitConfig {
    v1.filters
        .as_ref()
        .and_then(|f| f.rate_limiter.as_ref())
        .map(|rl| RateLimitConfig {
            max_connections: rl
                .request_limit
                .unwrap_or_else(crate::defaults::rate_limit_max),
            window: rl
                .window_length
                .as_ref()
                .and_then(|s| humantime::parse_duration(s).ok())
                .unwrap_or_else(crate::defaults::rate_limit_window),
            ..RateLimitConfig::default()
        })
        .unwrap_or_default()
}

fn warn_dropped_global_filters(v1: &V1InfrarustConfig, warnings: &mut Warnings) {
    let Some(ref filters) = v1.filters else {
        return;
    };
    if filters.ip_filter.as_ref().is_some_and(|f| f.enabled) {
        warnings
            .warn("Global IP filter is per-server in V2, must be configured in each server .toml");
    }
    if filters.id_filter.as_ref().is_some_and(|f| f.enabled) {
        warnings.warn("ID filter is not available in V2");
    }
    if filters.name_filter.as_ref().is_some_and(|f| f.enabled) {
        warnings.warn("Name filter is not available in V2");
    }
}

fn convert_ban(v1: &V1InfrarustConfig) -> BanConfig {
    v1.filters
        .as_ref()
        .and_then(|f| f.ban.as_ref())
        .map(|b| BanConfig {
            provider: crate::BanProviderSelection::Builtin,
            file: b
                .file_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(crate::defaults::ban_file),
            purge_interval: b
                .auto_cleanup_interval
                .map(Duration::from_secs)
                .unwrap_or_else(crate::defaults::ban_purge_interval),
            enable_audit_log: b.enable_audit_log.unwrap_or_else(crate::defaults::true_val),
            check_timeout: crate::defaults::ban_check_timeout(),
        })
        .unwrap_or_default()
}

fn convert_status_cache(v1: &V1InfrarustConfig) -> StatusCacheConfig {
    v1.cache
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
        .unwrap_or_default()
}

fn convert_telemetry(v1: &V1InfrarustConfig) -> Option<TelemetryConfig> {
    v1.telemetry.as_ref().map(|t| TelemetryConfig {
        enabled: t.enabled,
        endpoint: t.export_url.clone(),
        protocol: OtlpProtocol::default(),
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
    })
}

fn convert_receive_proxy_protocol(v1: &V1InfrarustConfig, warnings: &mut Warnings) -> bool {
    let receive = v1
        .proxy_protocol
        .as_ref()
        .and_then(|pp| pp.receive_enabled)
        .unwrap_or(false);

    if receive {
        warnings.warn(
            "receive_proxy_protocol now requires trusted_proxies: add the addresses \
             of your load balancers to the generated file or the proxy will not start",
        );
    }

    if let Some(pp) = &v1.proxy_protocol
        && (pp.receive_timeout_secs.is_some() || pp.receive_allowed_versions.is_some())
    {
        warnings
            .info("proxy_protocol.receive_timeout_secs and receive_allowed_versions are not in V2");
    }
    receive
}

fn convert_default_motd(v1: &V1InfrarustConfig) -> Option<MotdConfig> {
    let motds = v1.motds.as_ref()?;
    let entry = motds
        .unreachable
        .as_ref()
        .or(motds.unknown.as_ref())
        .or(motds.unable_status.as_ref())?;
    Some(MotdConfig {
        online: Some(motd_entry(entry)?),
        ..MotdConfig::default()
    })
}

fn convert_docker(v1: &V1InfrarustConfig) -> Option<DockerProviderConfig> {
    v1.docker_provider.as_ref().map(|dp| DockerProviderConfig {
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
        allowed_domains: Vec::new(),
        inherit_forwarding: false,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
        assert_eq!(result.config.name, Some("my-server".to_string()));
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.severity == MigrationSeverity::Warning
                    && w.message.contains("sanitized to 'my-server'")),
            "lossy sanitization must be surfaced as a warning"
        );
    }

    #[test]
    fn test_config_id_sanitization_collapses_dash_runs() {
        let mut v1 = minimal_v1();
        v1.config_id = Some("a  b--c".to_string());
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert_eq!(result.config.name, Some("a-b-c".to_string()));
    }

    #[test]
    fn test_config_id_without_usable_characters_is_dropped() {
        let mut v1 = minimal_v1();
        v1.config_id = Some("!!!".to_string());
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert_eq!(result.config.name, None);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.severity == MigrationSeverity::Warning)
        );
    }

    #[test]
    fn test_clean_config_id_does_not_warn() {
        let mut v1 = minimal_v1();
        v1.config_id = Some("survival-1@file_provider".to_string());
        let result = convert_v1_to_v2(&v1, "test.yaml");
        assert_eq!(result.config.name, Some("survival-1".to_string()));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_unparseable_v1_bind_warns_and_falls_back() {
        let v1: V1InfrarustConfig = serde_norway::from_str("bind: \"not an address\"").unwrap();
        let result = convert_v1_proxy_config(&v1);
        assert_eq!(result.config.bind, crate::defaults::bind());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.severity == MigrationSeverity::Warning
                    && w.message.contains("Cannot parse bind")),
            "dropped v1 bind must be surfaced as a warning"
        );
    }

    #[test]
    fn test_infrared_style_v1_bind_is_kept() {
        let v1: V1InfrarustConfig = serde_norway::from_str("bind: \":25577\"").unwrap();
        let result = convert_v1_proxy_config(&v1);
        assert_eq!(result.config.bind, "0.0.0.0:25577".parse().unwrap());
        assert!(result.warnings.is_empty());
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
    fn test_motd_protocol_version_is_migrated() {
        let mut v1 = minimal_v1();
        v1.motds = Some(V1Motds {
            online: Some(V1MotdEntry {
                enabled: true,
                text: Some("Online".to_string()),
                version_name: None,
                max_players: None,
                online_players: None,
                protocol_version: Some(47),
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
        assert_eq!(
            result.config.motd.online.unwrap().version_protocol,
            Some(47)
        );
        assert!(
            result
                .warnings
                .iter()
                .all(|w| !w.message.contains("were dropped")),
            "{:?}",
            result.warnings
        );
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
