use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// Transfer packet (Clientbound, 1.20.5+).
///
/// Transfers a player to another server without disconnection.
#[derive(Debug, Clone)]
pub struct CTransfer {
    pub host: String,
    pub port: i32,
}

impl Packet for CTransfer {
    const NAME: &'static str = "CTransfer";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let host = r.read_string()?;
        let port = r.read_var_int()?.0;
        Ok(Self { host, port })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.host)?;
        w.write_var_int(&VarInt(self.port))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_transfer_round_trip() {
        let pkt = CTransfer {
            host: "play.example.com".to_string(),
            port: 25565,
        };
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded = CTransfer::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.host, "play.example.com");
        assert_eq!(decoded.port, 25565);
    }
}
