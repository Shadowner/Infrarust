//! Player trait — the primary interface for interacting with connected players.

use std::net::SocketAddr;

use crate::error::PlayerError;
use crate::event::BoxFuture;
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
    /// Returns the player's unique session ID.
    fn id(&self) -> PlayerId;

    /// Returns the player's authenticated game profile.
    fn profile(&self) -> &GameProfile;

    /// Returns the Minecraft protocol version used by this player's client.
    fn protocol_version(&self) -> ProtocolVersion;

    /// Returns the player's remote (client) address.
    fn remote_addr(&self) -> SocketAddr;

    /// Returns the ID of the backend server the player is currently on, if any.
    fn current_server(&self) -> Option<ServerId>;

    /// Returns `true` if the player is still connected to the proxy.
    fn is_connected(&self) -> bool;

    /// Returns `true` if the player is on an active proxy path where
    /// packet injection and message sending are supported.
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

    /// Returns `true` if the player has the given permission.
    fn has_permission(&self, permission: &str) -> bool;
}
