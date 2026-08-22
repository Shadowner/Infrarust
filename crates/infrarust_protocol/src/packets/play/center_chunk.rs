use crate::codec::varint::VarInt;
use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone)]
pub struct CSetCenterChunk {
    pub chunk_x: i32,
    pub chunk_z: i32,
}

impl Packet for CSetCenterChunk {
    const NAME: &'static str = "CSetCenterChunk";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_14   => 0x40,
        V1_15   => 0x41,
        V1_16   => 0x40,
        V1_17   => 0x49,
        V1_18   => 0x4A,
        V1_19   => 0x48,
        V1_19_1 => 0x4B,
        V1_19_3 => 0x4A,
        V1_19_4 => 0x4E,
        V1_20_2 => 0x50,
        V1_20_3 => 0x52,
        V1_20_5 => 0x54,
        V1_21_2 => 0x58,
        V1_21_5 => 0x57,
        V1_21_9 => 0x5C,
    ];

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
