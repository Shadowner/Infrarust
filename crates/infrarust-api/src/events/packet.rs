//! Raw packet events (Tier 3).

use crate::event::{Event, ResultedEvent};
use crate::types::{PlayerId, RawPacket};

/// Direction of packet flow relative to the proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PacketDirection {
    /// Client → Server (through proxy).
    Serverbound,
    /// Server → Client (through proxy).
    Clientbound,
}

/// Fired when a raw packet passes through the proxy.
///
/// This is a Tier 3 event for plugins that need packet-level inspection
/// or modification. Listeners can pass, modify, or drop the packet.
pub struct RawPacketEvent {
    /// The player this packet belongs to.
    pub player_id: PlayerId,
    /// The direction of the packet.
    pub direction: PacketDirection,
    /// The original packet.
    pub packet: RawPacket,
    result: RawPacketResult,
}

impl RawPacketEvent {
    /// Creates a new `RawPacketEvent`.
    pub fn new(player_id: PlayerId, direction: PacketDirection, packet: RawPacket) -> Self {
        Self {
            player_id,
            direction,
            packet,
            result: RawPacketResult::default(),
        }
    }

    /// Shortcut: drop the packet.
    pub fn drop_packet(&mut self) {
        self.result = RawPacketResult::Drop;
    }

    /// Shortcut: replace the packet with a modified version.
    pub fn modify(&mut self, packet: RawPacket) {
        self.result = RawPacketResult::Modify { packet };
    }
}

/// The result of a [`RawPacketEvent`].
#[derive(Default)]
#[non_exhaustive]
pub enum RawPacketResult {
    /// Pass the packet through unmodified.
    #[default]
    Pass,
    /// Replace the packet with a modified version.
    Modify {
        /// The replacement packet.
        packet: RawPacket,
    },
    /// Drop the packet entirely.
    Drop,
}

impl Event for RawPacketEvent {}
impl ResultedEvent for RawPacketEvent {
    type Result = RawPacketResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn default_passes() {
        let event = RawPacketEvent::new(
            PlayerId::new(1),
            PacketDirection::Serverbound,
            RawPacket::new(0x00, bytes::Bytes::new()),
        );
        assert!(matches!(event.result(), RawPacketResult::Pass));
    }

    #[test]
    fn drop_packet() {
        let mut event = RawPacketEvent::new(
            PlayerId::new(1),
            PacketDirection::Clientbound,
            RawPacket::new(0x01, bytes::Bytes::new()),
        );
        event.drop_packet();
        assert!(matches!(event.result(), RawPacketResult::Drop));
    }

    #[test]
    fn direction_non_exhaustive() {
        let dir = PacketDirection::Serverbound;
        #[allow(unreachable_patterns)]
        match dir {
            PacketDirection::Serverbound | PacketDirection::Clientbound | _ => {}
        }
    }
}
