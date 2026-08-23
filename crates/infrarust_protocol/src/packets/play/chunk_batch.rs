use crate::codec::varint::VarInt;
use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone)]
pub struct CChunkBatchStart;

impl Packet for CChunkBatchStart {
    const NAME: &'static str = "CChunkBatchStart";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_20_2 => 0x0D,
        V1_21_5 => 0x0C,
    ];

    fn decode(_r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
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

#[derive(Debug, Clone)]
pub struct CChunkBatchFinished {
    pub batch_size: i32,
}

impl Packet for CChunkBatchFinished {
    const NAME: &'static str = "CChunkBatchFinished";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_20_2 => 0x0C,
        V1_21_5 => 0x0B,
    ];

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let batch_size = r.read_var_int()?.0;
        Ok(Self { batch_size })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&VarInt(self.batch_size))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn batch_start_round_trip() {
        let pkt = CChunkBatchStart;
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        assert!(buf.is_empty());
        CChunkBatchStart::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
    }

    #[test]
    fn batch_finished_round_trip() {
        let pkt = CChunkBatchFinished { batch_size: 1 };
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded =
            CChunkBatchFinished::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.batch_size, 1);
    }
}
