use crate::error::ProtocolResult;
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone)]
pub struct CStartConfiguration;

impl Packet for CStartConfiguration {
    const NAME: &'static str = "CStartConfiguration";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_20_2 => 0x65,
        V1_20_3 => 0x67,
        V1_20_5 => 0x69,
        V1_21_2 => 0x70,
        V1_21_5 => 0x6F,
        V1_21_9 => 0x74,
        V26_1   => 0x76,
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
pub struct SAcknowledgeConfiguration;

impl Packet for SAcknowledgeConfiguration {
    const NAME: &'static str = "SAcknowledgeConfiguration";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_20_2 => 0x0B,
        V1_20_5 => 0x0C,
        V1_21_2 => 0x0E,
        V1_21_6 => 0x0F,
        V26_1   => 0x10,
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
