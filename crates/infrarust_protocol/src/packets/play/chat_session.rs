//! Chat Session Update packet (Serverbound, Play).
//!
//! Contains the player's signed chat session key (Mojang signature).
//! Dropped by the proxy in intercepted modes because offline backends
//! can't validate the signature (UUID mismatch).

use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Chat Session Update — sent by the client to inform the server of
/// its signed chat session key.
///
/// The proxy never inspects this packet's contents. It only needs the
/// packet ID (via the registry) to identify and drop it before it
/// reaches an offline backend, so `decode` discards the body.
#[derive(Debug, Clone)]
pub struct SChatSessionUpdate;

impl Packet for SChatSessionUpdate {
    const NAME: &'static str = "SChatSessionUpdate";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        *r = &[];
        Ok(Self)
    }

    fn encode(
        &self,
        _w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn decode_succeeds_and_discards_body() {
        let mut body: &[u8] = &[0x01, 0x02, 0x03, 0x04];
        SChatSessionUpdate::decode(&mut body, ProtocolVersion::V1_21).unwrap();
        assert!(body.is_empty());
    }
}
