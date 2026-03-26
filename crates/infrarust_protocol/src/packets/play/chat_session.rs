//! Chat Session Update packet (Serverbound, Play).
//!
//! Contains the player's signed chat session key (Mojang signature).
//! Dropped by the proxy in intercepted modes because offline backends
//! can't validate the signature (UUID mismatch).

use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Chat Session Update — sent by the client to inform the server of
/// its signed chat session key.
///
/// The proxy never decodes or encodes this packet. It only needs the
/// packet ID (via the registry) to identify and drop it before it
/// reaches an offline backend.
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

    fn decode(_r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        Err(ProtocolError::invalid(
            "SChatSessionUpdate is not decoded by the proxy",
        ))
    }

    fn encode(
        &self,
        _w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        Ok(())
    }
}
