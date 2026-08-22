use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

pub const START_WAITING_CHUNKS: u8 = 13;

#[derive(Debug, Clone)]
pub struct CGameEvent {
    pub event: u8,
    pub value: f32,
}

impl Packet for CGameEvent {
    const NAME: &'static str = "CGameEvent";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_9    => 0x1B,
        V1_13   => 0x1E,
        V1_14   => 0x1D,
        V1_15   => 0x1E,
        V1_16   => 0x1D,
        V1_16_2 => 0x1C,
        V1_17   => 0x1D,
        V1_19   => 0x1B,
        V1_19_1 => 0x1D,
        V1_19_3 => 0x1C,
        V1_19_4 => 0x20,
        V1_20_5 => 0x22,
        V1_21_2 => 0x23,
        V1_21_5 => 0x22,
        V1_21_9 => 0x26,
    ];

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
