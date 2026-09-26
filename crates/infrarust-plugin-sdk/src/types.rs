use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use infrarust_plugin_common::enums::{
    BackendState, ConnectCause, FilterPriority, HandshakeIntent, LoginStage, MessagePhase,
    ResourcePackStatus, SessionEndReason, TransferOrigin, UnknownDomainBehavior,
};

use crate::bindings::ban_service as wb;
use crate::bindings::codec_registry as wc;
use crate::bindings::events as we;
use crate::bindings::guest as wg;
use crate::bindings::proxy_info as wi;
use crate::bindings::types as wt;
use crate::player::Player;

pub(crate) trait FromWit<W>: Sized {
    fn from_wit(w: W) -> Self;
}

pub(crate) trait ToWit<W> {
    fn to_wit(&self) -> W;
}

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

pub use infrarust_plugin_common::enums::ServerState;

pub use infrarust_plugin_common::enums::ProxyMode;

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

pub use infrarust_plugin_common::enums::PacketDirection;

pub use infrarust_plugin_common::enums::{ChatMode, MainHand, ParticleStatus};

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
            chat_mode: ChatMode::from_wit(settings.chat_mode),
            chat_colors: settings.chat_colors,
            skin_parts: settings.skin_parts,
            main_hand: MainHand::from_wit(settings.main_hand),
            text_filtering: settings.text_filtering,
            allow_listing: settings.allow_listing,
            particle_status: ParticleStatus::from_wit(settings.particle_status),
        }
    }
}

pub use infrarust_plugin_common::capability::Capability;

impl FromWit<wt::ServerState> for ServerState {
    fn from_wit(w: wt::ServerState) -> Self {
        match w {
            wt::ServerState::Online => Self::Online,
            wt::ServerState::Offline => Self::Offline,
            wt::ServerState::Starting => Self::Starting,
            wt::ServerState::Stopping => Self::Stopping,
            wt::ServerState::Sleeping => Self::Sleeping,
            wt::ServerState::Crashed => Self::Crashed,
        }
    }
}

impl FromWit<wt::ProxyMode> for ProxyMode {
    fn from_wit(w: wt::ProxyMode) -> Self {
        match w {
            wt::ProxyMode::Passthrough => Self::Passthrough,
            wt::ProxyMode::ZeroCopy => Self::ZeroCopy,
            wt::ProxyMode::ClientOnly => Self::ClientOnly,
            wt::ProxyMode::Offline => Self::Offline,
            wt::ProxyMode::ServerOnly => Self::ServerOnly,
        }
    }
}

impl FromWit<wt::PacketDirection> for PacketDirection {
    fn from_wit(w: wt::PacketDirection) -> Self {
        match w {
            wt::PacketDirection::Serverbound => Self::Serverbound,
            wt::PacketDirection::Clientbound => Self::Clientbound,
        }
    }
}

impl ToWit<wt::PacketDirection> for PacketDirection {
    fn to_wit(&self) -> wt::PacketDirection {
        match self {
            Self::Clientbound => wt::PacketDirection::Clientbound,
            _ => wt::PacketDirection::Serverbound,
        }
    }
}

impl FromWit<wt::ChatMode> for ChatMode {
    fn from_wit(w: wt::ChatMode) -> Self {
        match w {
            wt::ChatMode::Enabled => Self::Enabled,
            wt::ChatMode::CommandsOnly => Self::CommandsOnly,
            wt::ChatMode::Hidden => Self::Hidden,
        }
    }
}

impl FromWit<wt::MainHand> for MainHand {
    fn from_wit(w: wt::MainHand) -> Self {
        match w {
            wt::MainHand::Left => Self::Left,
            wt::MainHand::Right => Self::Right,
        }
    }
}

impl FromWit<wt::ParticleStatus> for ParticleStatus {
    fn from_wit(w: wt::ParticleStatus) -> Self {
        match w {
            wt::ParticleStatus::All => Self::All,
            wt::ParticleStatus::Decreased => Self::Decreased,
            wt::ParticleStatus::Minimal => Self::Minimal,
        }
    }
}

impl FromWit<wt::Capability> for Capability {
    fn from_wit(w: wt::Capability) -> Self {
        match w {
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

impl FromWit<wi::UnknownDomainBehavior> for UnknownDomainBehavior {
    fn from_wit(w: wi::UnknownDomainBehavior) -> Self {
        match w {
            wi::UnknownDomainBehavior::DefaultMotd => Self::DefaultMotd,
            wi::UnknownDomainBehavior::Drop => Self::Drop,
        }
    }
}

impl FromWit<we::MessagePhase> for MessagePhase {
    fn from_wit(w: we::MessagePhase) -> Self {
        match w {
            we::MessagePhase::Configuration => Self::Configuration,
            we::MessagePhase::Play => Self::Play,
        }
    }
}

impl FromWit<we::HandshakeIntent> for HandshakeIntent {
    fn from_wit(w: we::HandshakeIntent) -> Self {
        match w {
            we::HandshakeIntent::Status => Self::Status,
            we::HandshakeIntent::Login => Self::Login,
            we::HandshakeIntent::Transfer => Self::Transfer,
        }
    }
}

impl FromWit<we::ConnectCause> for ConnectCause {
    fn from_wit(w: we::ConnectCause) -> Self {
        match w {
            we::ConnectCause::Initial => Self::Initial,
            we::ConnectCause::Switch => Self::Switch,
            we::ConnectCause::LimboExit => Self::LimboExit,
            we::ConnectCause::KickRedirect => Self::KickRedirect,
            we::ConnectCause::PluginMessage => Self::PluginMessage,
        }
    }
}

impl FromWit<we::TransferOrigin> for TransferOrigin {
    fn from_wit(w: we::TransferOrigin) -> Self {
        match w {
            we::TransferOrigin::Plugin => Self::Plugin,
            we::TransferOrigin::Backend => Self::Backend,
        }
    }
}

impl FromWit<we::BackendState> for BackendState {
    fn from_wit(w: we::BackendState) -> Self {
        match w {
            we::BackendState::Healthy => Self::Healthy,
            we::BackendState::Probing => Self::Probing,
            we::BackendState::Unhealthy => Self::Unhealthy,
            we::BackendState::Draining => Self::Draining,
        }
    }
}

impl FromWit<wb::LoginStage> for LoginStage {
    fn from_wit(w: wb::LoginStage) -> Self {
        match w {
            wb::LoginStage::Status => Self::Status,
            wb::LoginStage::PreAuth => Self::PreAuth,
            wb::LoginStage::PostAuth => Self::PostAuth,
        }
    }
}

impl FromWit<wg::SessionEndReason> for SessionEndReason {
    fn from_wit(w: wg::SessionEndReason) -> Self {
        match w {
            wg::SessionEndReason::Disconnected => Self::Disconnected,
            wg::SessionEndReason::Released => Self::Released,
            wg::SessionEndReason::Kicked => Self::Kicked,
            wg::SessionEndReason::Redirected => Self::Redirected,
            wg::SessionEndReason::TimedOut => Self::TimedOut,
            wg::SessionEndReason::Shutdown => Self::Shutdown,
        }
    }
}

impl ToWit<wc::FilterPriority> for FilterPriority {
    fn to_wit(&self) -> wc::FilterPriority {
        match self {
            Self::First => wc::FilterPriority::First,
            Self::Early => wc::FilterPriority::Early,
            Self::Normal => wc::FilterPriority::Normal,
            Self::Late => wc::FilterPriority::Late,
            Self::Last => wc::FilterPriority::Last,
        }
    }
}

impl FromWit<we::ResourcePackStatus> for ResourcePackStatus {
    fn from_wit(w: we::ResourcePackStatus) -> Self {
        match w {
            we::ResourcePackStatus::SuccessfullyLoaded => Self::SuccessfullyLoaded,
            we::ResourcePackStatus::Declined => Self::Declined,
            we::ResourcePackStatus::FailedDownload => Self::FailedDownload,
            we::ResourcePackStatus::Accepted => Self::Accepted,
            we::ResourcePackStatus::Downloaded => Self::Downloaded,
            we::ResourcePackStatus::InvalidUrl => Self::InvalidUrl,
            we::ResourcePackStatus::FailedReload => Self::FailedReload,
            we::ResourcePackStatus::Discarded => Self::Discarded,
            we::ResourcePackStatus::Unknown(id) => Self::Unknown(id),
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

pub(crate) fn millis_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, millis)
}

pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn server_ids(ids: Vec<String>) -> Vec<ServerId> {
    ids.into_iter().map(ServerId).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
            Capability::from_wit(wt::Capability::PluginMessaging).to_kebab(),
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
