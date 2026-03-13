//! Virtual backend handler trait.

use crate::event::BoxFuture;
use crate::types::{PlayerId, RawPacket};

use super::session::VirtualBackendSession;

/// A handler for a virtual backend — a proxy-hosted "server" that speaks
/// raw Minecraft packets directly to the client.
///
/// Virtual backends are Tier 3 plugins that take full control of the
/// client connection. They must handle the Minecraft protocol themselves,
/// including sending join-game packets, chunks, and keep-alive responses.
///
/// # Implementation Notes
///
/// - `on_session_start` **must** send a `JoinGame` packet and initial world
///   data, or the client will disconnect.
/// - `on_packet_received` **must** handle `KeepAlive` packets.
///
/// # Example
/// ```ignore
/// struct MyVirtualBackend;
///
/// impl VirtualBackendHandler for MyVirtualBackend {
///     fn name(&self) -> &str { "my-vb" }
///
///     fn on_session_start(&self, session: &dyn VirtualBackendSession) -> BoxFuture<'_, ()> {
///         Box::pin(async move {
///             // Send JoinGame, spawn position, chunks...
///         })
///     }
///
///     fn on_packet_received(&self, session: &dyn VirtualBackendSession, packet: &RawPacket) -> BoxFuture<'_, ()> {
///         Box::pin(async move {
///             // Handle incoming packets
///         })
///     }
///
///     fn on_session_end(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
///         Box::pin(async {})
///     }
/// }
/// ```
pub trait VirtualBackendHandler: Send + Sync {
    /// Returns the name of this virtual backend handler.
    fn name(&self) -> &str;

    /// Called when a player session starts on this virtual backend.
    ///
    /// The handler **must** send the initial game state (`JoinGame` packet,
    /// spawn position, initial chunks) or the client will disconnect.
    fn on_session_start(&self, session: &dyn VirtualBackendSession) -> BoxFuture<'_, ()>;

    /// Called when a packet is received from the client.
    ///
    /// The handler **must** respond to `KeepAlive` packets to keep
    /// the connection alive.
    fn on_packet_received(
        &self,
        session: &dyn VirtualBackendSession,
        packet: &RawPacket,
    ) -> BoxFuture<'_, ()>;

    /// Called when the player session ends (disconnect or server switch).
    fn on_session_end(&self, player_id: PlayerId) -> BoxFuture<'_, ()>;
}
