use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, SystemTime};

use uuid::Uuid;

use crate::bindings::events as we;
use crate::bindings::{
    ban_service as wb, config_service as wc, load_balancer as wl, messaging as wm,
    plugin_registry as wr, proxy_info as wi, server_manager as ws,
};
use crate::error::Error;
use crate::event::BackendState;
use crate::plugin::PluginDependency;
use crate::types::{
    Capability, ChannelId, PlayerId, ProxyMode, ServerAddress, ServerId, ServerState, ip_from_wit,
    ip_to_wit, millis, socket_from_wit, time_from_millis, uuid_from_wit, uuid_to_wit,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub server: ServerId,
    pub state: ServerState,
}

pub struct Servers;

impl Servers {
    pub fn state(server: &ServerId) -> Result<Option<ServerState>, Error> {
        Ok(ws::get_state(server.as_str())?.map(ServerState::from_wit))
    }

    pub fn start(server: &ServerId) -> Result<(), Error> {
        Ok(ws::start(server.as_str())?)
    }

    pub fn stop(server: &ServerId) -> Result<(), Error> {
        Ok(ws::stop(server.as_str())?)
    }

    pub fn list() -> Result<Vec<ServerStatus>, Error> {
        Ok(ws::list()?
            .into_iter()
            .map(|status| ServerStatus {
                server: ServerId::from(status.server),
                state: ServerState::from_wit(status.state),
            })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BanTarget {
    Ip(IpAddr),
    IpRange(String),
    Username(String),
    Uuid(Uuid),
}

impl BanTarget {
    pub(crate) fn to_wit(&self) -> wb::BanTarget {
        match self {
            Self::Ip(ip) => wb::BanTarget::Ip(ip_to_wit(*ip)),
            Self::IpRange(range) => wb::BanTarget::IpRange(range.clone()),
            Self::Username(name) => wb::BanTarget::Username(name.clone()),
            Self::Uuid(uuid) => wb::BanTarget::Uuid(uuid_to_wit(*uuid)),
        }
    }

    pub(crate) fn from_wit(target: wb::BanTarget) -> Self {
        match target {
            wb::BanTarget::Ip(ip) => Self::Ip(ip_from_wit(ip)),
            wb::BanTarget::IpRange(range) => Self::IpRange(range),
            wb::BanTarget::Username(name) => Self::Username(name),
            wb::BanTarget::Uuid(uuid) => Self::Uuid(uuid_from_wit(uuid)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanRequest {
    pub target: BanTarget,
    pub reason: Option<String>,
    pub duration: Option<Duration>,
    pub kick: bool,
    pub silent: bool,
}

impl BanRequest {
    #[must_use]
    pub const fn new(target: BanTarget) -> Self {
        Self {
            target,
            reason: None,
            duration: None,
            kick: true,
            silent: false,
        }
    }

    #[must_use]
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    #[must_use]
    pub const fn kick(mut self, kick: bool) -> Self {
        self.kick = kick;
        self
    }

    #[must_use]
    pub const fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }

    pub(crate) fn from_wit(request: wb::BanRequest) -> Self {
        Self {
            target: BanTarget::from_wit(request.target),
            reason: request.reason,
            duration: request.duration_ms.map(Duration::from_millis),
            kick: request.kick,
            silent: request.silent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanEntry {
    pub id: String,
    pub target: BanTarget,
    pub reason: Option<String>,
    pub source: String,
    pub created_at: SystemTime,
    pub expires_at: Option<SystemTime>,
}

impl BanEntry {
    pub(crate) fn from_wit(entry: wb::BanEntry) -> Self {
        Self {
            id: entry.id,
            target: BanTarget::from_wit(entry.target),
            reason: entry.reason,
            source: entry.source,
            created_at: time_from_millis(entry.created_at),
            expires_at: entry.expires_at.map(time_from_millis),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanPage {
    pub entries: Vec<BanEntry>,
    pub next_cursor: Option<String>,
}

pub struct Bans;

impl Bans {
    pub fn ban(request: BanRequest) -> Result<BanEntry, Error> {
        let request = wb::BanRequest {
            target: request.target.to_wit(),
            reason: request.reason,
            duration_ms: request.duration.map(millis),
            kick: request.kick,
            silent: request.silent,
        };
        Ok(BanEntry::from_wit(wb::ban(&request)?))
    }

    pub fn unban(target: &BanTarget) -> Result<Option<BanEntry>, Error> {
        Ok(wb::unban(&target.to_wit())?.map(BanEntry::from_wit))
    }

    pub fn get(target: &BanTarget) -> Result<Option<BanEntry>, Error> {
        Ok(wb::get(&target.to_wit())?.map(BanEntry::from_wit))
    }

    pub fn is_banned(target: &BanTarget) -> Result<bool, Error> {
        Ok(Self::get(target)?.is_some())
    }

    pub fn list(cursor: Option<&str>, limit: u32) -> Result<BanPage, Error> {
        let page = wb::list(cursor, limit)?;
        Ok(BanPage {
            entries: page.entries.into_iter().map(BanEntry::from_wit).collect(),
            next_cursor: page.next_cursor,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerConfig {
    pub id: ServerId,
    pub network: Option<String>,
    pub addresses: Vec<ServerAddress>,
    pub domains: Vec<String>,
    pub proxy_mode: ProxyMode,
    pub limbo_handlers: Vec<String>,
    pub max_players: u32,
    pub disconnect_message: Option<String>,
    pub send_proxy_protocol: bool,
    pub has_server_manager: bool,
}

impl ServerConfig {
    fn from_wit(config: wc::ServerConfig) -> Self {
        Self {
            id: ServerId::from(config.id),
            network: config.network,
            addresses: config
                .addresses
                .into_iter()
                .map(ServerAddress::from_wit)
                .collect(),
            domains: config.domains,
            proxy_mode: ProxyMode::from_wit(config.proxy_mode),
            limbo_handlers: config.limbo_handlers,
            max_players: config.max_players,
            disconnect_message: config.disconnect_message,
            send_proxy_protocol: config.send_proxy_protocol,
            has_server_manager: config.has_server_manager,
        }
    }
}

pub struct Config;

impl Config {
    pub fn get(key: &str) -> Result<Option<String>, Error> {
        Ok(wc::get_value(key)?)
    }

    pub fn server(server: &ServerId) -> Result<Option<ServerConfig>, Error> {
        Ok(wc::get_server(server.as_str())?.map(ServerConfig::from_wit))
    }

    pub fn servers() -> Result<Vec<ServerConfig>, Error> {
        Ok(wc::list_servers()?
            .into_iter()
            .map(ServerConfig::from_wit)
            .collect())
    }

    pub fn server_document(server: &ServerId) -> Result<Option<String>, Error> {
        Ok(wc::get_server_document(server.as_str())?)
    }

    pub fn server_sources() -> Result<Vec<ServerSource>, Error> {
        Ok(wc::list_server_sources()?
            .into_iter()
            .map(|source| ServerSource {
                id: source.id,
                provider_id: source.provider_id,
                provider_type: source.provider_type,
                editable: source.editable,
            })
            .collect())
    }

    pub fn proxy_document() -> Result<String, Error> {
        Ok(wc::get_proxy_config_document()?)
    }

    pub fn effective_proxy_document() -> Result<String, Error> {
        Ok(wc::get_effective_proxy_config_document()?)
    }

    pub fn write_proxy_document(document: &str) -> Result<(), Error> {
        Ok(wc::write_proxy_config_document(document)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerSource {
    pub id: String,
    pub provider_id: String,
    pub provider_type: String,
    pub editable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BackendStatus {
    pub address: ServerAddress,
    pub weight: u32,
    pub effective_weight: u32,
    pub state: BackendState,
    pub active_connections: u64,
    pub healthy_since: Option<Duration>,
    pub ejections: u32,
    pub last_failure_ago: Option<Duration>,
}

impl BackendStatus {
    fn from_wit(status: wl::BackendStatus) -> Self {
        Self {
            address: ServerAddress::from_wit(status.address),
            weight: status.weight,
            effective_weight: status.effective_weight,
            state: match status.state {
                we::BackendState::Healthy => BackendState::Healthy,
                we::BackendState::Probing => BackendState::Probing,
                we::BackendState::Unhealthy => BackendState::Unhealthy,
                we::BackendState::Draining => BackendState::Draining,
            },
            active_connections: status.active_connections,
            healthy_since: status.healthy_since_secs.map(Duration::from_secs),
            ejections: status.ejections,
            last_failure_ago: status.last_failure_secs_ago.map(Duration::from_secs),
        }
    }
}

pub struct LoadBalancer;

impl LoadBalancer {
    pub fn strategy(server: &ServerId) -> Result<Option<String>, Error> {
        Ok(wl::strategy(server.as_str())?)
    }

    pub fn backends(server: &ServerId) -> Result<Vec<BackendStatus>, Error> {
        Ok(wl::backends(server.as_str())?
            .into_iter()
            .map(BackendStatus::from_wit)
            .collect())
    }

    pub fn set_drained(
        server: &ServerId,
        address: &ServerAddress,
        drained: bool,
    ) -> Result<(), Error> {
        Ok(wl::set_drained(
            server.as_str(),
            &address.to_wit(),
            drained,
        )?)
    }

    pub fn reset_backend(server: &ServerId, address: &ServerAddress) -> Result<(), Error> {
        Ok(wl::reset_backend(server.as_str(), &address.to_wit())?)
    }
}

pub struct Messaging;

impl Messaging {
    pub fn register(channel: &ChannelId) -> Result<(), Error> {
        Ok(wm::register_channel(&channel.to_wit())?)
    }

    pub fn unregister(channel: &ChannelId) -> Result<bool, Error> {
        Ok(wm::unregister_channel(&channel.to_wit())?)
    }

    pub fn channels() -> Result<Vec<ChannelId>, Error> {
        Ok(wm::channels()?
            .into_iter()
            .map(ChannelId::from_wit)
            .collect())
    }

    pub fn send_to_player(player: PlayerId, channel: &ChannelId, data: &[u8]) -> Result<(), Error> {
        Ok(wm::send_to_player(
            player.as_u64(),
            &channel.to_wit(),
            data,
        )?)
    }

    pub fn send_to_backend(
        player: PlayerId,
        channel: &ChannelId,
        data: &[u8],
    ) -> Result<(), Error> {
        Ok(wm::send_to_backend(
            player.as_u64(),
            &channel.to_wit(),
            data,
        )?)
    }

    pub fn send_to_server(
        server: &ServerId,
        channel: &ChannelId,
        data: &[u8],
    ) -> Result<u32, Error> {
        Ok(wm::send_to_server(
            server.as_str(),
            &channel.to_wit(),
            data,
        )?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnknownDomainBehavior {
    DefaultMotd,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RateLimitInfo {
    pub max_connections: u32,
    pub window: Duration,
    pub status_max: u32,
    pub status_window: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusCacheInfo {
    pub ttl: Duration,
    pub max_entries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeepaliveInfo {
    pub time: Duration,
    pub interval: Duration,
    pub retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProxyDetails {
    pub version: String,
    pub bind: SocketAddr,
    pub max_connections: u32,
    pub connect_timeout: Duration,
    pub receive_proxy_protocol: bool,
    pub worker_threads: u32,
    pub so_reuseport: bool,
    pub rate_limit: RateLimitInfo,
    pub status_cache: StatusCacheInfo,
    pub keepalive: KeepaliveInfo,
    pub telemetry_enabled: bool,
    pub docker_enabled: bool,
    pub web_api_enabled: bool,
    pub web_ui_enabled: bool,
    pub unknown_domain_behavior: UnknownDomainBehavior,
}

impl ProxyDetails {
    fn from_wit(details: wi::ProxyDetails) -> Self {
        Self {
            version: details.version,
            bind: socket_from_wit(details.bind),
            max_connections: details.max_connections,
            connect_timeout: Duration::from_millis(details.connect_timeout_ms),
            receive_proxy_protocol: details.receive_proxy_protocol,
            worker_threads: details.worker_threads,
            so_reuseport: details.so_reuseport,
            rate_limit: RateLimitInfo {
                max_connections: details.rate_limit.max_connections,
                window: Duration::from_millis(details.rate_limit.window_ms),
                status_max: details.rate_limit.status_max,
                status_window: Duration::from_millis(details.rate_limit.status_window_ms),
            },
            status_cache: StatusCacheInfo {
                ttl: Duration::from_millis(details.status_cache.ttl_ms),
                max_entries: details.status_cache.max_entries,
            },
            keepalive: KeepaliveInfo {
                time: Duration::from_millis(details.keepalive.time_ms),
                interval: Duration::from_millis(details.keepalive.interval_ms),
                retries: details.keepalive.retries,
            },
            telemetry_enabled: details.telemetry_enabled,
            docker_enabled: details.docker_enabled,
            web_api_enabled: details.web_api_enabled,
            web_ui_enabled: details.web_ui_enabled,
            unknown_domain_behavior: match details.unknown_domain_behavior {
                wi::UnknownDomainBehavior::DefaultMotd => UnknownDomainBehavior::DefaultMotd,
                wi::UnknownDomainBehavior::Drop => UnknownDomainBehavior::Drop,
            },
        }
    }
}

pub struct Proxy;

impl Proxy {
    #[must_use]
    pub fn details() -> ProxyDetails {
        ProxyDetails::from_wit(wi::details())
    }

    #[must_use]
    pub fn version() -> String {
        wi::details().version
    }

    #[must_use]
    pub fn granted_capabilities() -> Vec<Capability> {
        wi::granted_capabilities()
            .into_iter()
            .map(Capability::from_wit)
            .collect()
    }

    #[must_use]
    pub fn has_capability(capability: Capability) -> bool {
        Self::granted_capabilities().contains(&capability)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub state: String,
    pub dependencies: Vec<PluginDependency>,
}

impl PluginInfo {
    fn from_wit(info: wr::PluginInfo) -> Self {
        Self {
            id: info.id,
            name: info.name,
            version: info.version,
            authors: info.authors,
            description: info.description,
            state: info.state,
            dependencies: info.dependencies,
        }
    }
}

pub struct Plugins;

impl Plugins {
    #[must_use]
    pub fn list() -> Vec<PluginInfo> {
        wr::list().into_iter().map(PluginInfo::from_wit).collect()
    }

    #[must_use]
    pub fn get(id: &str) -> Option<PluginInfo> {
        wr::get(id).map(PluginInfo::from_wit)
    }

    #[must_use]
    pub fn is_loaded(id: &str) -> bool {
        Self::get(id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ban_targets_round_trip() {
        for target in [
            BanTarget::Ip("203.0.113.7".parse().unwrap()),
            BanTarget::IpRange("10.0.0.0/8".into()),
            BanTarget::Username("Steve".into()),
            BanTarget::Uuid(Uuid::from_u128(7)),
        ] {
            assert_eq!(BanTarget::from_wit(target.to_wit()), target);
        }
    }

    #[test]
    fn a_ban_request_kicks_by_default() {
        let request = BanRequest::new(BanTarget::Username("Steve".into()))
            .reason("griefing")
            .duration(Duration::from_secs(60));
        assert!(request.kick);
        assert!(!request.silent);
        assert_eq!(request.duration.map(millis), Some(60_000));
    }
}
