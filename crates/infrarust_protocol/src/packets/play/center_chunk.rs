//! Set Center Chunk packet (Clientbound, 1.14+).
//!
//! Tells the client which chunk is at the center of the view. Required
//! before sending chunk data so the client knows which chunks to render.

use crate::codec::varint::VarInt;
use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Set Center Chunk packet (Clientbound).
#[derive(Debug, Clone)]
pub struct CSetCenterChunk {
    /// Chunk X coordinate.
    pub chunk_x: i32,
    /// Chunk Z coordinate.
    pub chunk_z: i32,
}

impl Packet for CSetCenterChunk {
    const NAME: &'static str = "CSetCenterChunk";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let chunk_x = r.read_var_int()?.0;
        let chunk_z = r.read_var_int()?.0;
        Ok(Self { chunk_x, chunk_z })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&VarInt(self.chunk_x))?;
        w.write_var_int(&VarInt(self.chunk_z))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn round_trip() {
        let pkt = CSetCenterChunk {
            chunk_x: 0,
            chunk_z: 0,
        };
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded = CSetCenterChunk::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.chunk_x, 0);
        assert_eq!(decoded.chunk_z, 0);
    }
}
