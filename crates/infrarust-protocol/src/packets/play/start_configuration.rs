use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Start configuration packet (Clientbound, Play state).
///
/// Empty signal sent by the proxy during a server switch (1.20.2+) to tell the
/// client to re-enter the configuration phase.
#[derive(Debug, Clone)]
pub struct CStartConfiguration;

impl Packet for CStartConfiguration {
    const NAME: &'static str = "CStartConfiguration";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

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

/// Acknowledge configuration packet (Serverbound, Play state).
///
/// Empty confirmation from the client that it has entered configuration phase.
/// This is the Play-state acknowledgment (distinct from `SAcknowledgeFinishConfig`
/// which is Config-state).
#[derive(Debug, Clone)]
pub struct SAcknowledgeConfiguration;

impl Packet for SAcknowledgeConfiguration {
    const NAME: &'static str = "SAcknowledgeConfiguration";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

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
