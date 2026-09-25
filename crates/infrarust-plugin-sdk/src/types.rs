use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::bindings::types as wt;
use crate::player::Player;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(u64);

impl PlayerId {
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u64> for PlayerId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServerId(String);

impl ServerId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ServerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ServerId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl From<String> for ServerId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&String> for ServerId {
    fn from(id: &String) -> Self {
        Self(id.clone())
    }
}

impl PartialEq<str> for ServerId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for ServerId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerRef {
    pub id: PlayerId,
    pub uuid: Uuid,
    pub username: String,
}

impl PlayerRef {
    #[must_use]
    pub const fn handle(&self) -> Player {
        Player::new(self.id)
    }

    pub(crate) fn from_wit(player: wt::PlayerRef) -> Self {
        Self {
            id: PlayerId(player.id),
            uuid: uuid_from_wit(player.uuid),
            username: player.username,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProfile {
    pub uuid: Uuid,
    pub username: String,
    pub properties: Vec<ProfileProperty>,
}

impl GameProfile {
    pub(crate) fn from_wit(profile: wt::GameProfile) -> Self {
        Self {
            uuid: uuid_from_wit(profile.uuid),
            username: profile.username,
            properties: profile
                .properties
                .into_iter()
                .map(|property| ProfileProperty {
                    name: property.name,
                    value: property.value,
                    signature: property.signature,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl ServerAddress {
    pub(crate) fn from_wit(address: wt::ServerAddress) -> Self {
        Self {
            host: address.host,
            port: address.port,
        }
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerState {
    Online,
    Offline,
    Starting,
    Stopping,
    Sleeping,
    Crashed,
}

impl ServerState {
    pub(crate) const fn from_wit(state: wt::ServerState) -> Self {
        match state {
            wt::ServerState::Online => Self::Online,
            wt::ServerState::Offline => Self::Offline,
            wt::ServerState::Starting => Self::Starting,
            wt::ServerState::Stopping => Self::Stopping,
            wt::ServerState::Sleeping => Self::Sleeping,
            wt::ServerState::Crashed => Self::Crashed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProxyMode {
    Passthrough,
    ZeroCopy,
    ClientOnly,
    Offline,
    ServerOnly,
}

impl ProxyMode {
    pub(crate) const fn from_wit(mode: wt::ProxyMode) -> Self {
        match mode {
            wt::ProxyMode::Passthrough => Self::Passthrough,
            wt::ProxyMode::ZeroCopy => Self::ZeroCopy,
            wt::ProxyMode::ClientOnly => Self::ClientOnly,
            wt::ProxyMode::Offline => Self::Offline,
            wt::ProxyMode::ServerOnly => Self::ServerOnly,
        }
    }
}

pub(crate) fn uuid_from_wit(uuid: wt::Uuid) -> Uuid {
    Uuid::from_u64_pair(uuid.hi, uuid.lo)
}

pub(crate) fn uuid_to_wit(uuid: Uuid) -> wt::Uuid {
    let (hi, lo) = uuid.as_u64_pair();
    wt::Uuid { hi, lo }
}

pub(crate) fn ip_from_wit(ip: wt::IpAddress) -> IpAddr {
    match ip {
        wt::IpAddress::Ipv4((a, b, c, d)) => IpAddr::V4(Ipv4Addr::new(a, b, c, d)),
        wt::IpAddress::Ipv6((a, b, c, d, e, f, g, h)) => {
            IpAddr::V6(Ipv6Addr::new(a, b, c, d, e, f, g, h))
        }
    }
}

pub(crate) fn ip_to_wit(ip: IpAddr) -> wt::IpAddress {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, d] = v4.octets();
            wt::IpAddress::Ipv4((a, b, c, d))
        }
        IpAddr::V6(v6) => {
            let [a, b, c, d, e, f, g, h] = v6.segments();
            wt::IpAddress::Ipv6((a, b, c, d, e, f, g, h))
        }
    }
}

pub(crate) fn socket_from_wit(address: wt::SocketAddress) -> SocketAddr {
    SocketAddr::new(ip_from_wit(address.ip), address.port)
}

pub(crate) fn time_from_millis(millis: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_millis(millis))
        .unwrap_or(UNIX_EPOCH)
}

pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn server_ids(ids: Vec<String>) -> Vec<ServerId> {
    ids.into_iter().map(ServerId).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_cross_the_boundary_as_two_halves() {
        let uuid: Uuid = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0".parse().unwrap();
        let wire = uuid_to_wit(uuid);
        assert_eq!(wire.hi, 0x0f1e_2d3c_4b5a_6978);
        assert_eq!(wire.lo, 0x8796_a5b4_c3d2_e1f0);
        assert_eq!(uuid_from_wit(wire), uuid);
    }

    #[test]
    fn addresses_and_times_convert_to_std_types() {
        for ip in ["203.0.113.7", "2001:db8::7"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert_eq!(ip_from_wit(ip_to_wit(ip)), ip);
        }
        let socket = socket_from_wit(wt::SocketAddress {
            ip: wt::IpAddress::Ipv4((127, 0, 0, 1)),
            port: 25565,
        });
        assert_eq!(socket, "127.0.0.1:25565".parse().unwrap());
        assert_eq!(
            time_from_millis(1_500),
            UNIX_EPOCH + Duration::from_millis(1_500)
        );
        assert_eq!(millis(Duration::from_secs(2)), 2_000);
    }

    #[test]
    fn server_ids_compare_with_strings() {
        let lobby = ServerId::from("lobby");
        assert_eq!(lobby, "lobby");
        assert_eq!(lobby.to_string(), "lobby");
        assert_eq!(PlayerId::new(3).as_u64(), 3);
    }
}
