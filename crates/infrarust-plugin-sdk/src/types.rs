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
    pub(crate) fn to_wit(&self) -> wt::GameProfile {
        wt::GameProfile {
            uuid: uuid_to_wit(self.uuid),
            username: self.username.clone(),
            properties: self
                .properties
                .iter()
                .map(|property| wt::ProfileProperty {
                    name: property.name.clone(),
                    value: property.value.clone(),
                    signature: property.signature.clone(),
                })
                .collect(),
        }
    }

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
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    pub(crate) fn from_wit(address: wt::ServerAddress) -> Self {
        Self {
            host: address.host,
            port: address.port,
        }
    }

    pub(crate) fn to_wit(&self) -> wt::ServerAddress {
        wt::ServerAddress {
            host: self.host.clone(),
            port: self.port,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelId {
    modern: Option<String>,
    legacy: Option<String>,
}

impl ChannelId {
    #[must_use]
    pub fn modern(id: impl Into<String>) -> Self {
        Self {
            modern: Some(id.into()),
            legacy: None,
        }
    }

    #[must_use]
    pub fn legacy(name: impl Into<String>) -> Self {
        Self {
            modern: None,
            legacy: Some(name.into()),
        }
    }

    #[must_use]
    pub fn pair(modern: impl Into<String>, legacy: impl Into<String>) -> Self {
        Self {
            modern: Some(modern.into()),
            legacy: Some(legacy.into()),
        }
    }

    #[must_use]
    pub fn bungeecord() -> Self {
        Self::pair("bungeecord:main", "BungeeCord")
    }

    #[must_use]
    pub fn modern_id(&self) -> Option<&str> {
        self.modern.as_deref()
    }

    #[must_use]
    pub fn legacy_name(&self) -> Option<&str> {
        self.legacy.as_deref()
    }

    #[must_use]
    pub fn matches(&self, raw: &str) -> bool {
        self.modern_id() == Some(raw) || self.legacy_name() == Some(raw)
    }

    pub(crate) fn to_wit(&self) -> wt::ChannelId {
        wt::ChannelId {
            modern: self.modern.clone(),
            legacy: self.legacy.clone(),
        }
    }

    pub(crate) fn from_wit(channel: wt::ChannelId) -> Self {
        Self {
            modern: channel.modern,
            legacy: channel.legacy,
        }
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.modern, &self.legacy) {
            (Some(modern), Some(legacy)) => write!(f, "{modern} ({legacy})"),
            (Some(name), None) | (None, Some(name)) => f.write_str(name),
            (None, None) => f.write_str("-"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PacketDirection {
    Serverbound,
    Clientbound,
}

impl PacketDirection {
    pub(crate) const fn from_wit(direction: wt::PacketDirection) -> Self {
        match direction {
            wt::PacketDirection::Serverbound => Self::Serverbound,
            wt::PacketDirection::Clientbound => Self::Clientbound,
        }
    }

    pub(crate) const fn to_wit(self) -> wt::PacketDirection {
        match self {
            Self::Serverbound => wt::PacketDirection::Serverbound,
            Self::Clientbound => wt::PacketDirection::Clientbound,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChatMode {
    Enabled,
    CommandsOnly,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MainHand {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParticleStatus {
    All,
    Decreased,
    Minimal,
}

pub use crate::bindings::types::SkinParts;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientSettings {
    pub locale: String,
    pub view_distance: u8,
    pub chat_mode: ChatMode,
    pub chat_colors: bool,
    pub skin_parts: SkinParts,
    pub main_hand: MainHand,
    pub text_filtering: bool,
    pub allow_listing: bool,
    pub particle_status: ParticleStatus,
}

impl ClientSettings {
    pub(crate) fn from_wit(settings: wt::ClientSettings) -> Self {
        Self {
            locale: settings.locale,
            view_distance: settings.view_distance,
            chat_mode: match settings.chat_mode {
                wt::ChatMode::Enabled => ChatMode::Enabled,
                wt::ChatMode::CommandsOnly => ChatMode::CommandsOnly,
                wt::ChatMode::Hidden => ChatMode::Hidden,
            },
            chat_colors: settings.chat_colors,
            skin_parts: settings.skin_parts,
            main_hand: match settings.main_hand {
                wt::MainHand::Left => MainHand::Left,
                wt::MainHand::Right => MainHand::Right,
            },
            text_filtering: settings.text_filtering,
            allow_listing: settings.allow_listing,
            particle_status: match settings.particle_status {
                wt::ParticleStatus::All => ParticleStatus::All,
                wt::ParticleStatus::Decreased => ParticleStatus::Decreased,
                wt::ParticleStatus::Minimal => ParticleStatus::Minimal,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    EventBus,
    PlayerRead,
    PlayerWrite,
    RawPacket,
    ServerManage,
    Ban,
    Command,
    Scheduler,
    ConfigRead,
    ConfigWrite,
    CodecFilter,
    TransportFilter,
    Limbo,
    VirtualBackend,
    PermissionProvider,
    FilesystemExtended,
    Network,
    ChatIntercept,
    BanProvider,
    PluginMessaging,
}

impl Capability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EventBus => "event-bus",
            Self::PlayerRead => "player-read",
            Self::PlayerWrite => "player-write",
            Self::RawPacket => "raw-packet",
            Self::ServerManage => "server-manage",
            Self::Ban => "ban",
            Self::Command => "command",
            Self::Scheduler => "scheduler",
            Self::ConfigRead => "config-read",
            Self::ConfigWrite => "config-write",
            Self::CodecFilter => "codec-filter",
            Self::TransportFilter => "transport-filter",
            Self::Limbo => "limbo",
            Self::VirtualBackend => "virtual-backend",
            Self::PermissionProvider => "permission-provider",
            Self::FilesystemExtended => "filesystem-extended",
            Self::Network => "network",
            Self::ChatIntercept => "chat-intercept",
            Self::BanProvider => "ban-provider",
            Self::PluginMessaging => "plugin-messaging",
        }
    }

    pub(crate) const fn from_wit(capability: wt::Capability) -> Self {
        match capability {
            wt::Capability::EventBus => Self::EventBus,
            wt::Capability::PlayerRead => Self::PlayerRead,
            wt::Capability::PlayerWrite => Self::PlayerWrite,
            wt::Capability::RawPacket => Self::RawPacket,
            wt::Capability::ServerManage => Self::ServerManage,
            wt::Capability::Ban => Self::Ban,
            wt::Capability::Command => Self::Command,
            wt::Capability::Scheduler => Self::Scheduler,
            wt::Capability::ConfigRead => Self::ConfigRead,
            wt::Capability::ConfigWrite => Self::ConfigWrite,
            wt::Capability::CodecFilter => Self::CodecFilter,
            wt::Capability::TransportFilter => Self::TransportFilter,
            wt::Capability::Limbo => Self::Limbo,
            wt::Capability::VirtualBackend => Self::VirtualBackend,
            wt::Capability::PermissionProvider => Self::PermissionProvider,
            wt::Capability::FilesystemExtended => Self::FilesystemExtended,
            wt::Capability::Network => Self::Network,
            wt::Capability::ChatIntercept => Self::ChatIntercept,
            wt::Capability::BanProvider => Self::BanProvider,
            wt::Capability::PluginMessaging => Self::PluginMessaging,
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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
    fn a_channel_keeps_the_names_it_was_built_with() {
        let pair = ChannelId::pair("myplugin:main", "MyPlugin");
        assert!(pair.matches("MyPlugin"));
        assert!(pair.matches("myplugin:main"));
        assert_eq!(pair.to_string(), "myplugin:main (MyPlugin)");
        assert_eq!(ChannelId::from_wit(pair.to_wit()), pair);
        assert_eq!(ChannelId::modern("a:b").legacy_name(), None);
        assert_eq!(ChannelId::bungeecord().legacy_name(), Some("BungeeCord"));
    }

    #[test]
    fn capabilities_have_their_config_names() {
        assert_eq!(
            Capability::from_wit(wt::Capability::PluginMessaging).as_str(),
            "plugin-messaging"
        );
        assert_eq!(Capability::ChatIntercept.to_string(), "chat-intercept");
    }

    #[test]
    fn server_ids_compare_with_strings() {
        let lobby = ServerId::from("lobby");
        assert_eq!(lobby, "lobby");
        assert_eq!(lobby.to_string(), "lobby");
        assert_eq!(PlayerId::new(3).as_u64(), 3);
    }
}
