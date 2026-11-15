use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::Instant;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::play::keepalive::{CKeepAlive, SKeepAlive};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::error::CoreError;
use crate::util::packets::encode_packet;

const KEEPALIVE_TIMEOUT_SECS: u64 = 15;

pub(crate) enum KeepAliveTick {
    Send(PacketFrame),
    Idle,
    Timeout,
}

struct Pending {
    id: i64,
    sent_at: Instant,
}

pub(crate) struct KeepAliveState {
    pending: Option<Pending>,
}

impl KeepAliveState {
    pub const fn new() -> Self {
        Self { pending: None }
    }

    pub fn tick(
        &mut self,
        version: ProtocolVersion,
        registry: &PacketRegistry,
    ) -> Result<KeepAliveTick, CoreError> {
        if let Some(pending) = &self.pending {
            if pending.sent_at.elapsed() >= Duration::from_secs(KEEPALIVE_TIMEOUT_SECS) {
                return Ok(KeepAliveTick::Timeout);
            }
            return Ok(KeepAliveTick::Idle);
        }

        let id = if version.less_than(ProtocolVersion::V1_12_2) {
            i64::from(rand::random::<i32>())
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64
        };

        self.pending = Some(Pending {
            id,
            sent_at: Instant::now(),
        });

        let frame = encode_packet(&CKeepAlive { id }, version, registry)?;
        Ok(KeepAliveTick::Send(frame))
    }

    pub fn on_response(&mut self, id: i64) -> Option<Duration> {
        let pending = self.pending.take_if(|pending| pending.id == id)?;
        Some(pending.sent_at.elapsed())
    }
}

pub(crate) fn is_keepalive_response(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> bool {
    let expected_id = registry.get_packet_id::<SKeepAlive>(version);
    Some(frame.id) == expected_id
}

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
    fn tick_sends_on_first_call() {
        let registry = test_registry();
        let mut state = KeepAliveState::new();

        let result = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        assert!(
            matches!(result, KeepAliveTick::Send(_)),
            "first tick should produce a frame"
        );
        assert!(state.pending.is_some());
    }

    #[test]
    fn second_tick_without_response_is_idle_not_resend() {
        // Regression: previously every tick reset `last_sent_at` and re-sent a
        // ping, so the timeout could never accumulate. A second tick while still
        // awaiting a response must NOT send another ping — it must idle so the
        // timer keeps running. No backdating required to expose the old bug.
        let registry = test_registry();
        let mut state = KeepAliveState::new();

        assert!(matches!(
            state.tick(ProtocolVersion::V1_21, &registry).unwrap(),
            KeepAliveTick::Send(_)
        ));
        let result = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        assert!(
            matches!(result, KeepAliveTick::Idle),
            "a second tick with no response must idle, not resend"
        );
    }

    #[test]
    fn tick_times_out_after_grace_without_response() {
        let registry = test_registry();
        let mut state = KeepAliveState::new();

        let _ = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        // Advance time past the timeout while still awaiting the response.
        state.pending.as_mut().unwrap().sent_at =
            Instant::now() - Duration::from_secs(KEEPALIVE_TIMEOUT_SECS + 1);

        let result = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        assert!(
            matches!(result, KeepAliveTick::Timeout),
            "an unanswered keepalive past the grace window must time out"
        );
    }

    #[test]
    fn tick_resends_after_a_valid_response() {
        let registry = test_registry();
        let mut state = KeepAliveState::new();

        let _ = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        let sent = state.pending.as_ref().unwrap().id;
        assert!(state.on_response(sent).is_some(), "valid id clears pending");
        assert!(state.pending.is_none());

        let result = state.tick(ProtocolVersion::V1_21, &registry).unwrap();
        assert!(
            matches!(result, KeepAliveTick::Send(_)),
            "after a confirmed response the next tick sends a fresh ping"
        );
    }

    #[test]
    fn on_response_correct_id() {
        let mut state = KeepAliveState::new();
        state.pending = Some(Pending {
            id: 42,
            sent_at: Instant::now(),
        });

        assert!(state.on_response(42).is_some());
        assert!(state.pending.is_none());
        assert!(
            state.on_response(42).is_none(),
            "a second answer measures nothing"
        );
    }

    #[test]
    fn on_response_wrong_id() {
        let mut state = KeepAliveState::new();
        state.pending = Some(Pending {
            id: 42,
            sent_at: Instant::now(),
        });

        assert!(state.on_response(99).is_none());
        assert!(state.pending.is_some());
    }
}
