//! Virtual backend session trait.

use crate::error::PlayerError;
use crate::event::BoxFuture;
use crate::types::{Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId};

pub mod private {
    /// Sealed — only the proxy implements [`VirtualBackendSession`](super::VirtualBackendSession).
    pub trait Sealed {}
}

/// A session handle for a player connected to a virtual backend.
///
/// Provides full packet-level control over the client connection.
/// Obtained in [`VirtualBackendHandler`](super::handler::VirtualBackendHandler) callbacks.
pub trait VirtualBackendSession: Send + Sync + private::Sealed {
    /// Returns the player's session ID.
    fn player_id(&self) -> PlayerId;

    /// Returns the player's game profile.
    fn profile(&self) -> &GameProfile;

    /// Returns the player's protocol version.
    fn protocol_version(&self) -> ProtocolVersion;

    /// Sends a raw packet to the client.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::SendFailed)` if the packet could not be delivered.
    fn send_packet(&self, packet: &RawPacket) -> Result<(), PlayerError>;

    /// Sends a chat message to the player (convenience wrapper).
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::SendFailed)` if the message could not be delivered.
    fn send_message(&self, message: Component) -> Result<(), PlayerError>;

    /// Switches the player to a real backend server.
    fn switch_server(&self, target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>>;

    /// Disconnects the player with a reason message.
    fn disconnect(&self, reason: Component) -> BoxFuture<'_, ()>;
}
