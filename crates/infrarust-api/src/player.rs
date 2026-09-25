//! Player trait — the primary interface for interacting with connected players.

use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use bytes::Bytes;

use crate::error::PlayerError;
use crate::event::BoxFuture;
use crate::messaging::ChannelId;
use crate::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};

pub mod private {
    /// Sealed — only the proxy implements [`Player`](super::Player).
    pub trait Sealed {}
}

/// A player connected to the proxy.
///
/// Obtained from [`PlayerRegistry`](crate::services::player_registry::PlayerRegistry)
/// as `Arc<dyn Player>`. The proxy is the sole implementor.
///
/// # Active vs Passive Mode
///
/// Some methods only work when the player is on an **active** proxy path
/// (`ClientOnly`, Offline, or Full mode). In passive modes (Passthrough,
/// `ZeroCopy`), methods like `send_message` or `switch_server` will return
/// `Err(PlayerError::NotActive)`.
///
/// Use [`is_active()`](Player::is_active) to check before calling these methods.
pub trait Player: Send + Sync + private::Sealed {
    fn id(&self) -> PlayerId;

    fn profile(&self) -> &GameProfile;

    fn protocol_version(&self) -> ProtocolVersion;

    fn remote_addr(&self) -> SocketAddr;

    /// `None` if the player hasn't been routed to a backend yet.
    fn current_server(&self) -> Option<ServerId>;

    fn is_connected(&self) -> bool;

    /// Active means the proxy path supports packet injection and
    /// message sending (ClientOnly, Offline, or Full mode).
    fn is_active(&self) -> bool;

    /// Disconnects the player from the proxy with a reason message.
    ///
    /// This always works regardless of the proxy mode.
    fn disconnect(&self, reason: Component) -> BoxFuture<'_, ()>;

    /// Sends a chat message to the player.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::NotActive)` if the player is on a passive
    /// proxy path, or `Err(PlayerError::Disconnected)` if not connected.
    fn send_message(&self, message: Component) -> Result<(), PlayerError>;

    /// Sends a title display to the player.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::NotActive)` in passive mode.
    fn send_title(&self, title: TitleData) -> Result<(), PlayerError>;

    /// Sends an action bar message to the player.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::NotActive)` in passive mode.
    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError>;

    /// Sends a raw packet to the player's client.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::NotActive)` in passive mode.
    fn send_packet(&self, packet: RawPacket) -> Result<(), PlayerError>;

    /// Switches the player to a different backend server.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::NotActive)` in passive mode, or
    /// `Err(PlayerError::ServerNotFound)` if the target doesn't exist.
    fn switch_server(&self, target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>>;

    fn is_online_mode(&self) -> bool;

    fn has_permission(&self, permission: &str) -> bool;

    fn refresh_permissions(&self) -> BoxFuture<'_, ()>;

    fn connected_at(&self) -> SystemTime;

    fn virtual_host(&self) -> Option<String> {
        None
    }

    fn client_brand(&self) -> Option<String> {
        None
    }

    fn settings(&self) -> Option<ClientSettings> {
        None
    }

    fn known_channels(&self) -> Vec<String> {
        Vec::new()
    }

    fn ping(&self) -> Option<Duration> {
        None
    }

    fn send_plugin_message(&self, _channel: &ChannelId, _data: Bytes) -> Result<(), PlayerError> {
        Err(PlayerError::NotActive)
    }

    fn send_plugin_message_to_backend(
        &self,
        _channel: &ChannelId,
        _data: Bytes,
    ) -> Result<(), PlayerError> {
        Err(PlayerError::NotActive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ChatMode {
    #[default]
    Enabled,
    CommandsOnly,
    Hidden,
}

impl ChatMode {
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::CommandsOnly,
            2 => Self::Hidden,
            _ => Self::Enabled,
        }
    }

    pub const fn id(self) -> i32 {
        match self {
            Self::Enabled => 0,
            Self::CommandsOnly => 1,
            Self::Hidden => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MainHand {
    Left,
    #[default]
    Right,
}

impl MainHand {
    pub const fn from_id(id: i32) -> Self {
        if id == 0 { Self::Left } else { Self::Right }
    }

    pub const fn id(self) -> i32 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ParticleStatus {
    #[default]
    All,
    Decreased,
    Minimal,
}

impl ParticleStatus {
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Decreased,
            2 => Self::Minimal,
            _ => Self::All,
        }
    }

    pub const fn id(self) -> i32 {
        match self {
            Self::All => 0,
            Self::Decreased => 1,
            Self::Minimal => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkinParts(u8);

impl SkinParts {
    pub const CAPE: u8 = 0x01;
    pub const JACKET: u8 = 0x02;
    pub const LEFT_SLEEVE: u8 = 0x04;
    pub const RIGHT_SLEEVE: u8 = 0x08;
    pub const LEFT_PANTS: u8 = 0x10;
    pub const RIGHT_PANTS: u8 = 0x20;
    pub const HAT: u8 = 0x40;
    pub const ALL: Self = Self(0x7F);

    pub const fn new(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn shows(self, part: u8) -> bool {
        self.0 & part == part
    }

    pub const fn cape(self) -> bool {
        self.shows(Self::CAPE)
    }

    pub const fn jacket(self) -> bool {
        self.shows(Self::JACKET)
    }

    pub const fn left_sleeve(self) -> bool {
        self.shows(Self::LEFT_SLEEVE)
    }

    pub const fn right_sleeve(self) -> bool {
        self.shows(Self::RIGHT_SLEEVE)
    }

    pub const fn left_pants(self) -> bool {
        self.shows(Self::LEFT_PANTS)
    }

    pub const fn right_pants(self) -> bool {
        self.shows(Self::RIGHT_PANTS)
    }

    pub const fn hat(self) -> bool {
        self.shows(Self::HAT)
    }
}

impl Default for SkinParts {
    fn default() -> Self {
        Self::ALL
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    pub fn new(locale: impl Into<String>) -> Self {
        Self {
            locale: locale.into(),
            view_distance: 10,
            chat_mode: ChatMode::Enabled,
            chat_colors: true,
            skin_parts: SkinParts::ALL,
            main_hand: MainHand::Right,
            text_filtering: false,
            allow_listing: true,
            particle_status: ParticleStatus::All,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_ids_round_trip() {
        for mode in [ChatMode::Enabled, ChatMode::CommandsOnly, ChatMode::Hidden] {
            assert_eq!(ChatMode::from_id(mode.id()), mode);
        }
        for hand in [MainHand::Left, MainHand::Right] {
            assert_eq!(MainHand::from_id(hand.id()), hand);
        }
        for status in [
            ParticleStatus::All,
            ParticleStatus::Decreased,
            ParticleStatus::Minimal,
        ] {
            assert_eq!(ParticleStatus::from_id(status.id()), status);
        }
        assert_eq!(ChatMode::from_id(9), ChatMode::Enabled);
    }

    #[test]
    fn skin_parts_read_their_bits() {
        let parts = SkinParts::new(SkinParts::CAPE | SkinParts::HAT);
        assert!(parts.cape());
        assert!(parts.hat());
        assert!(!parts.jacket());
        assert!(SkinParts::ALL.right_pants());
        assert_eq!(parts.bits(), 0x41);
    }
}
