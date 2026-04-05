//! Player trait — the primary interface for interacting with connected players.

use std::net::SocketAddr;
use std::time::SystemTime;

use crate::error::PlayerError;
use crate::event::BoxFuture;
use crate::permissions::PermissionLevel;
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

    fn permission_level(&self) -> PermissionLevel;

    fn has_permission(&self, permission: &str) -> bool;

    fn connected_at(&self) -> SystemTime;
}
