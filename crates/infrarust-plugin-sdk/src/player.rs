use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use uuid::Uuid;

use crate::bindings::players as wp;
use crate::bindings::types as wt;
use crate::component::{Component, from_host};
use crate::error::Error;
use crate::types::{
    ClientSettings, GameProfile, PlayerId, PlayerRef, ServerAddress, ServerId, socket_from_wit,
    time_from_millis, uuid_from_wit, uuid_to_wit,
};

pub use crate::bindings::players::{BossBarColor, BossBarFlags, BossBarOverlay};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ConnectionResult {
    Success,
    AlreadyConnected,
    Denied(Component),
    Failed(Component),
    Cancelled,
}

impl ConnectionResult {
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success | Self::AlreadyConnected)
    }

    fn from_wit(result: wp::ConnectionResult) -> Self {
        match result {
            wp::ConnectionResult::Success => Self::Success,
            wp::ConnectionResult::AlreadyConnected => Self::AlreadyConnected,
            wp::ConnectionResult::Denied(reason) => Self::Denied(from_host(reason)),
            wp::ConnectionResult::Failed(reason) => Self::Failed(from_host(reason)),
            wp::ConnectionResult::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    pub title: Component,
    pub progress: f32,
    pub color: BossBarColor,
    pub overlay: BossBarOverlay,
    pub flags: BossBarFlags,
}

impl BossBar {
    #[must_use]
    pub fn new(title: impl Into<Component>) -> Self {
        Self {
            title: title.into(),
            progress: 1.0,
            color: BossBarColor::Pink,
            overlay: BossBarOverlay::Progress,
            flags: BossBarFlags::empty(),
        }
    }

    #[must_use]
    pub const fn progress(mut self, progress: f32) -> Self {
        self.progress = progress;
        self
    }

    #[must_use]
    pub const fn color(mut self, color: BossBarColor) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub const fn overlay(mut self, overlay: BossBarOverlay) -> Self {
        self.overlay = overlay;
        self
    }

    #[must_use]
    pub const fn flags(mut self, flags: BossBarFlags) -> Self {
        self.flags = flags;
        self
    }

    fn to_wit(&self) -> wp::BossBar {
        wp::BossBar {
            title: self.title.to_arena(),
            progress: self.progress,
            color: self.color,
            overlay: self.overlay,
            flags: self.flags,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BossBarHandle {
    id: Uuid,
}

impl BossBarHandle {
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    pub fn set_title(&self, title: impl Into<Component>) -> Result<(), Error> {
        self.update(&wp::BossBarUpdate::Title(title.into().to_arena()))
    }

    pub fn set_progress(&self, progress: f32) -> Result<(), Error> {
        self.update(&wp::BossBarUpdate::Progress(progress))
    }

    pub fn set_style(&self, color: BossBarColor, overlay: BossBarOverlay) -> Result<(), Error> {
        self.update(&wp::BossBarUpdate::Style((color, overlay)))
    }

    pub fn set_flags(&self, flags: BossBarFlags) -> Result<(), Error> {
        self.update(&wp::BossBarUpdate::Flags(flags))
    }

    pub fn hide(self) -> Result<(), Error> {
        Ok(wp::hide_boss_bar(uuid_to_wit(self.id))?)
    }

    fn update(&self, update: &wp::BossBarUpdate) -> Result<(), Error> {
        Ok(wp::update_boss_bar(uuid_to_wit(self.id), update)?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourcePackRequest {
    pub id: Uuid,
    pub url: String,
    pub hash: Option<String>,
    pub required: bool,
    pub prompt: Option<Component>,
}

impl ResourcePackRequest {
    #[must_use]
    pub fn new(id: Uuid, url: impl Into<String>) -> Self {
        Self {
            id,
            url: url.into(),
            hash: None,
            required: false,
            prompt: None,
        }
    }

    #[must_use]
    pub fn hash(mut self, sha1_hex: impl Into<String>) -> Self {
        self.hash = Some(sha1_hex.into());
        self
    }

    #[must_use]
    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<Component>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    fn to_wit(&self) -> wp::ResourcePackRequest {
        wp::ResourcePackRequest {
            id: uuid_to_wit(self.id),
            url: self.url.clone(),
            hash: self.hash.clone(),
            required: self.required,
            prompt: self.prompt.as_ref().map(Component::to_arena),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TitleData {
    pub title: Component,
    pub subtitle: Component,
    pub fade_in_ticks: i32,
    pub stay_ticks: i32,
    pub fade_out_ticks: i32,
}

impl TitleData {
    #[must_use]
    pub fn new(title: impl Into<Component>, subtitle: impl Into<Component>) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            fade_in_ticks: 10,
            stay_ticks: 70,
            fade_out_ticks: 20,
        }
    }

    #[must_use]
    pub const fn fade_in(mut self, ticks: i32) -> Self {
        self.fade_in_ticks = ticks;
        self
    }

    #[must_use]
    pub const fn stay(mut self, ticks: i32) -> Self {
        self.stay_ticks = ticks;
        self
    }

    #[must_use]
    pub const fn fade_out(mut self, ticks: i32) -> Self {
        self.fade_out_ticks = ticks;
        self
    }

    pub(crate) fn to_wit(&self) -> wt::TitleData {
        wt::TitleData {
            title: self.title.to_arena(),
            subtitle: self.subtitle.to_arena(),
            fade_in_ticks: self.fade_in_ticks,
            stay_ticks: self.stay_ticks,
            fade_out_ticks: self.fade_out_ticks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerInfo {
    pub player: PlayerRef,
    pub profile: GameProfile,
    pub protocol: i32,
    pub remote_addr: SocketAddr,
    pub current_server: Option<ServerId>,
    pub online_mode: bool,
    pub connected: bool,
    pub active: bool,
    pub connected_at: SystemTime,
    pub virtual_host: Option<String>,
    pub client_brand: Option<String>,
    pub ping: Option<Duration>,
    pub settings: Option<ClientSettings>,
    pub known_channels: Vec<String>,
}

impl PlayerInfo {
    #[must_use]
    pub const fn id(&self) -> PlayerId {
        self.player.id
    }

    #[must_use]
    pub const fn handle(&self) -> Player {
        Player::new(self.player.id)
    }

    pub(crate) fn from_wit(info: wp::PlayerInfo) -> Self {
        Self {
            player: PlayerRef::from_wit(info.player),
            profile: GameProfile::from_wit(info.profile),
            protocol: info.protocol,
            remote_addr: socket_from_wit(info.remote_addr),
            current_server: info.current_server.map(ServerId::from),
            online_mode: info.online_mode,
            connected: info.connected,
            active: info.active,
            connected_at: time_from_millis(info.connected_at),
            virtual_host: info.virtual_host,
            client_brand: info.client_brand,
            ping: info.ping_ms.map(|ms| Duration::from_millis(u64::from(ms))),
            settings: info.settings.map(ClientSettings::from_wit),
            known_channels: info.known_channels,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Player {
    id: PlayerId,
}

impl From<PlayerId> for Player {
    fn from(id: PlayerId) -> Self {
        Self::new(id)
    }
}

impl Player {
    #[must_use]
    pub const fn new(id: PlayerId) -> Self {
        Self { id }
    }

    #[must_use]
    pub const fn id(&self) -> PlayerId {
        self.id
    }

    #[must_use]
    pub fn info(&self) -> Option<PlayerInfo> {
        Players::get(self.id)
    }

    pub fn send_message(&self, message: impl Into<Component>) -> Result<(), Error> {
        Ok(wp::send_message(
            self.id.as_u64(),
            &message.into().to_arena(),
        )?)
    }

    pub fn send_title(&self, title: &TitleData) -> Result<(), Error> {
        Ok(wp::send_title(self.id.as_u64(), &title.to_wit())?)
    }

    pub fn send_action_bar(&self, message: impl Into<Component>) -> Result<(), Error> {
        Ok(wp::send_action_bar(
            self.id.as_u64(),
            &message.into().to_arena(),
        )?)
    }

    pub fn send_packet(&self, packet_id: i32, data: Vec<u8>) -> Result<(), Error> {
        Ok(wp::send_packet(
            self.id.as_u64(),
            &wt::RawPacket { packet_id, data },
        )?)
    }

    pub fn disconnect(&self, reason: impl Into<Component>) -> Result<(), Error> {
        Ok(wp::disconnect(self.id.as_u64(), &reason.into().to_arena())?)
    }

    pub fn switch_server(&self, server: impl Into<ServerId>) -> Result<(), Error> {
        Ok(wp::switch_server(self.id.as_u64(), server.into().as_str())?)
    }

    pub fn has_permission(&self, permission: &str) -> Result<bool, Error> {
        Ok(wp::has_permission(self.id.as_u64(), permission)?)
    }

    pub fn connect(&self, server: impl Into<ServerId>) -> Result<ConnectionResult, Error> {
        Ok(ConnectionResult::from_wit(wp::connect(
            self.id.as_u64(),
            server.into().as_str(),
        )?))
    }

    pub fn set_player_list_header_footer(
        &self,
        header: impl Into<Component>,
        footer: impl Into<Component>,
    ) -> Result<(), Error> {
        Ok(wp::set_player_list_header_footer(
            self.id.as_u64(),
            &header.into().to_arena(),
            &footer.into().to_arena(),
        )?)
    }

    pub fn clear_title(&self) -> Result<(), Error> {
        Ok(wp::clear_title(self.id.as_u64(), false)?)
    }

    pub fn reset_title(&self) -> Result<(), Error> {
        Ok(wp::clear_title(self.id.as_u64(), true)?)
    }

    pub fn show_boss_bar(&self, bar: &BossBar) -> Result<BossBarHandle, Error> {
        let id = wp::show_boss_bar(self.id.as_u64(), &bar.to_wit())?;
        Ok(BossBarHandle {
            id: uuid_from_wit(id),
        })
    }

    pub fn send_resource_pack(&self, pack: &ResourcePackRequest) -> Result<(), Error> {
        Ok(wp::send_resource_pack(self.id.as_u64(), &pack.to_wit())?)
    }

    pub fn remove_resource_pack(&self, id: Option<Uuid>) -> Result<(), Error> {
        Ok(wp::remove_resource_pack(
            self.id.as_u64(),
            id.map(uuid_to_wit),
        )?)
    }

    pub fn transfer(&self, host: impl Into<String>, port: u16) -> Result<(), Error> {
        let target = ServerAddress::new(host, port);
        Ok(wp::transfer(self.id.as_u64(), &target.to_wit())?)
    }

    pub fn store_cookie(&self, key: &str, data: &[u8]) -> Result<(), Error> {
        Ok(wp::store_cookie(self.id.as_u64(), key, data)?)
    }

    pub fn request_cookie(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        Ok(wp::request_cookie(self.id.as_u64(), key)?)
    }

    pub fn refresh_permissions(&self) -> Result<(), Error> {
        Ok(wp::refresh_permissions(self.id.as_u64())?)
    }
}

pub struct Players;

impl Players {
    #[must_use]
    pub fn get(id: PlayerId) -> Option<PlayerInfo> {
        wp::get(id.as_u64()).map(PlayerInfo::from_wit)
    }

    #[must_use]
    pub fn by_name(username: &str) -> Option<PlayerInfo> {
        wp::get_by_name(username).map(PlayerInfo::from_wit)
    }

    #[must_use]
    pub fn by_uuid(uuid: Uuid) -> Option<PlayerInfo> {
        wp::get_by_uuid(uuid_to_wit(uuid)).map(PlayerInfo::from_wit)
    }

    #[must_use]
    pub fn list() -> Vec<PlayerInfo> {
        wp::list(None)
            .into_iter()
            .map(PlayerInfo::from_wit)
            .collect()
    }

    #[must_use]
    pub fn on_server(server: &ServerId) -> Vec<PlayerInfo> {
        wp::list(Some(server.as_str()))
            .into_iter()
            .map(PlayerInfo::from_wit)
            .collect()
    }

    #[must_use]
    pub fn count() -> u32 {
        wp::count(None)
    }

    #[must_use]
    pub fn count_on(server: &ServerId) -> u32 {
        wp::count(Some(server.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_info_converts_to_std_types() {
        let info = PlayerInfo::from_wit(wp::PlayerInfo {
            player: wt::PlayerRef {
                id: 7,
                uuid: wt::Uuid { hi: 1, lo: 2 },
                username: "Steve".into(),
            },
            profile: wt::GameProfile {
                uuid: wt::Uuid { hi: 1, lo: 2 },
                username: "Steve".into(),
                properties: vec![],
            },
            protocol: 767,
            remote_addr: wt::SocketAddress {
                ip: wt::IpAddress::Ipv4((10, 0, 0, 1)),
                port: 40_000,
            },
            current_server: Some("lobby".into()),
            online_mode: true,
            connected: true,
            active: false,
            connected_at: 1_000,
            virtual_host: None,
            client_brand: Some("vanilla".into()),
            ping_ms: Some(42),
            settings: None,
            known_channels: vec!["test:echo".into()],
        });
        assert_eq!(info.known_channels, ["test:echo"]);
        assert_eq!(info.id(), PlayerId::new(7));
        assert_eq!(info.handle().id(), PlayerId::new(7));
        assert_eq!(info.player.uuid, Uuid::from_u64_pair(1, 2));
        assert_eq!(info.remote_addr, "10.0.0.1:40000".parse().unwrap());
        assert_eq!(info.current_server, Some(ServerId::from("lobby")));
        assert_eq!(info.ping, Some(Duration::from_millis(42)));
    }

    #[test]
    fn a_boss_bar_and_a_pack_cross_as_wit_records() {
        let bar = BossBar::new("Boss")
            .progress(0.5)
            .color(BossBarColor::Red)
            .flags(BossBarFlags::DARKEN_SCREEN | BossBarFlags::CREATE_WORLD_FOG);
        let wire = bar.to_wit();
        assert_eq!(wire.progress, 0.5);
        assert!(wire.flags.contains(BossBarFlags::CREATE_WORLD_FOG));
        assert_eq!(
            Component::from_arena(wire.title).unwrap(),
            Component::text("Boss")
        );

        let pack = ResourcePackRequest::new(Uuid::from_u128(3), "https://example.com/p.zip")
            .required(true)
            .prompt("please");
        let wire = pack.to_wit();
        assert!(wire.required);
        assert_eq!(wire.id, uuid_to_wit(Uuid::from_u128(3)));
        assert!(wire.prompt.is_some());
        assert!(ConnectionResult::from_wit(wp::ConnectionResult::AlreadyConnected).is_success());
    }

    #[test]
    fn a_title_carries_both_lines_as_arenas() {
        let title = TitleData::new("Hello", Component::text("world").bold()).stay(40);
        let wire = title.to_wit();
        assert_eq!(wire.stay_ticks, 40);
        assert_eq!(
            Component::from_arena(wire.subtitle).unwrap(),
            Component::text("world").bold()
        );
    }
}
