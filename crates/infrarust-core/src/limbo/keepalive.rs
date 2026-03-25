//! KeepAlive state machine for the Limbo loop.
//!
//! Tracks sent keepalive IDs, enforces the 15-second timeout, and provides
//! helpers for detecting and extracting serverbound keepalive responses.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::Instant;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::keepalive::{CKeepAlive, SKeepAlive};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::player::packets::encode_packet;

/// Maximum time (seconds) to wait for a keepalive response before
/// considering the client timed out.
const KEEPALIVE_TIMEOUT_SECS: u64 = 15;

/// Tracks the keepalive handshake with a single client.
pub(crate) struct KeepAliveState {
    last_sent_id: i64,
    last_sent_at: Instant,
    awaiting_response: bool,
}

impl KeepAliveState {
    pub fn new() -> Self {
        Self {
            last_sent_id: 0,
            last_sent_at: Instant::now(),
            awaiting_response: false,
        }
    }

    /// Generates a new keepalive frame if one should be sent.
    ///
    /// Returns `Some(frame)` with a fresh `CKeepAlive` packet, or `None` if
    /// the client has not responded within 15 seconds (timeout).
    pub fn tick(
        &mut self,
        version: ProtocolVersion,
        registry: &PacketRegistry,
    ) -> Result<Option<PacketFrame>, CoreError> {
        if self.awaiting_response
            && self.last_sent_at.elapsed() > Duration::from_secs(KEEPALIVE_TIMEOUT_SECS)
        {
            return Ok(None); // Client timed out
        }

        let id = if version.less_than(ProtocolVersion::V1_12_2) {
            i64::from(rand::random::<i32>())
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64
        };

        self.last_sent_id = id;
        self.last_sent_at = Instant::now();
        self.awaiting_response = true;

        let frame = encode_packet(&CKeepAlive { id }, version, registry)?;
        Ok(Some(frame))
    }

    /// Processes a keepalive response from the client.
    ///
    /// Returns `true` if the ID matches the last sent keepalive, clearing
    /// the pending state. Returns `false` on mismatch.
    pub fn on_response(&mut self, id: i64) -> bool {
        if id == self.last_sent_id {
            self.awaiting_response = false;
            true
        } else {
            false
        }
    }
}

/// Returns `true` if the frame is a serverbound `SKeepAlive` packet.
pub(crate) fn is_keepalive_response(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> bool {
    let expected_id = registry.get_packet_id::<SKeepAlive>(
        ConnectionState::Play,
        Direction::Serverbound,
        version,
    );
    Some(frame.id) == expected_id
}

/// Extracts the keepalive ID from a serverbound `SKeepAlive` frame.
///
/// Returns `None` if decoding fails.
pub(crate) fn extract_keepalive_id(frame: &PacketFrame, version: ProtocolVersion) -> Option<i64> {
    let mut data = frame.payload.as_ref();
    SKeepAlive::decode(&mut data, version)
        .ok()
        .map(|pkt| pkt.id)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::super::test_helpers::test_registry;
    use super::*;

    #[test]
    fn tick_returns_some_on_first_call() {
        let registry = test_registry();
        let mut state = KeepAliveState::new();

        let result = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        assert!(result.is_some(), "first tick should produce a frame");
        assert!(state.awaiting_response);
    }

    #[tokio::test]
    async fn tick_returns_none_after_timeout() {
        let registry = test_registry();
        let mut state = KeepAliveState::new();

        // Send a keepalive
        let _ = state.tick(ProtocolVersion::V1_21, &registry).unwrap();

        // Simulate timeout by backdating last_sent_at
        state.last_sent_at = Instant::now() - Duration::from_secs(KEEPALIVE_TIMEOUT_SECS + 1);

        let result = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        assert!(result.is_none(), "should return None after timeout");
    }

    #[test]
    fn on_response_correct_id() {
        let mut state = KeepAliveState::new();
        state.last_sent_id = 42;
        state.awaiting_response = true;

        assert!(state.on_response(42));
        assert!(!state.awaiting_response);
    }

    #[test]
    fn on_response_wrong_id() {
        let mut state = KeepAliveState::new();
        state.last_sent_id = 42;
        state.awaiting_response = true;

        assert!(!state.on_response(99));
        assert!(state.awaiting_response);
    }
}
