//! Game Event packet (Clientbound).
//!
//! Notifies the client of game state changes. Event 13 (`START_WAITING_CHUNKS`)
//! is critical for 1.20.2+ — without it the client stays on "Loading Terrain".

use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Game event IDs used by the Limbo engine.
pub const START_WAITING_CHUNKS: u8 = 13;

/// Game Event packet (Clientbound).
///
/// Used to signal game state changes to the client.
#[derive(Debug, Clone)]
pub struct CGameEvent {
    /// The event type ID.
    pub event: u8,
    /// Event-specific floating-point value.
    pub value: f32,
}

impl Packet for CGameEvent {
    const NAME: &'static str = "CGameEvent";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let event = r.read_u8()?;
        let value = r.read_f32_be()?;
        Ok(Self { event, value })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_u8(self.event)?;
        w.write_f32_be(self.value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn round_trip() {
        let pkt = CGameEvent {
            event: START_WAITING_CHUNKS,
            value: 0.0,
        };
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded = CGameEvent::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.event, START_WAITING_CHUNKS);
        assert!((decoded.value - 0.0).abs() < f32::EPSILON);
    }
}
