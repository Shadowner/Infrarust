use crate::error::ProtocolResult;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone)]
pub struct SChatSessionUpdate;

impl Packet for SChatSessionUpdate {
    const NAME: &'static str = "SChatSessionUpdate";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_21   => 0x07,
        V1_21_2 => 0x08,
        V1_21_6 => 0x09,
        V26_1   => 0x0A,
    ];

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
