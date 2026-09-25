use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use uuid::Uuid;

use crate::bindings::{ban_service as wb, config_service as wc, server_manager as ws};
use crate::error::Error;
use crate::types::{
    ProxyMode, ServerAddress, ServerId, ServerState, ip_from_wit, ip_to_wit, millis,
    time_from_millis, uuid_from_wit, uuid_to_wit,
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
    fn to_wit(&self) -> wb::BanTarget {
        match self {
            Self::Ip(ip) => wb::BanTarget::Ip(ip_to_wit(*ip)),
            Self::IpRange(range) => wb::BanTarget::IpRange(range.clone()),
            Self::Username(name) => wb::BanTarget::Username(name.clone()),
            Self::Uuid(uuid) => wb::BanTarget::Uuid(uuid_to_wit(*uuid)),
        }
    }

    fn from_wit(target: wb::BanTarget) -> Self {
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
    fn from_wit(entry: wb::BanEntry) -> Self {
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
