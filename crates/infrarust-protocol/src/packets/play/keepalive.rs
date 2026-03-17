use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::packets::Packet;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

// ── CKeepAlive ─────────────────────────────────────────────────────

/// Keep-alive packet (Clientbound).
///
/// Sent periodically by the server. The client must respond with `SKeepAlive`
/// containing the same ID within 15 seconds or get disconnected.
///
/// Wire format varies by version:
/// - 1.7.2 - 1.7.6: `i32`
/// - 1.8 - 1.12.1: `VarInt`
/// - 1.12.2+: `i64`
#[derive(Debug, Clone)]
pub struct CKeepAlive {
    pub id: i64,
}

impl Packet for CKeepAlive {
    const NAME: &'static str = "CKeepAlive";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let id = decode_keepalive_id(r, version)?;
        Ok(Self { id })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        encode_keepalive_id(w, self.id, version)
    }
}

// ── SKeepAlive ─────────────────────────────────────────────────────

/// Keep-alive packet (Serverbound).
///
/// Client's response to `CKeepAlive` with the same ID.
#[derive(Debug, Clone)]
pub struct SKeepAlive {
    pub id: i64,
}

impl Packet for SKeepAlive {
    const NAME: &'static str = "SKeepAlive";

    fn state() -> ConnectionState {
        ConnectionState::Play
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let id = decode_keepalive_id(r, version)?;
        Ok(Self { id })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        encode_keepalive_id(w, self.id, version)
    }
}

// ── Shared encode/decode ───────────────────────────────────────────

fn decode_keepalive_id(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<i64> {
    if version.no_less_than(ProtocolVersion::V1_12_2) {
        r.read_i64_be()
    } else if version.no_less_than(ProtocolVersion::V1_8) {
        Ok(i64::from(r.read_var_int()?.0))
    } else {
        Ok(i64::from(r.read_i32_be()?))
    }
}

fn encode_keepalive_id(
    mut w: &mut (impl std::io::Write + ?Sized),
    id: i64,
    version: ProtocolVersion,
) -> ProtocolResult<()> {
    if version.no_less_than(ProtocolVersion::V1_12_2) {
        w.write_i64_be(id)?;
    } else if version.no_less_than(ProtocolVersion::V1_8) {
        // Protocol keepalive IDs fit in i32 for pre-1.12.2
        w.write_var_int(&VarInt(id as i32))?;
    } else {
        w.write_i32_be(id as i32)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        P::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_keepalive_round_trip_i64() {
        let pkt = CKeepAlive {
            id: 0x1234_5678_9ABC_DEF0,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.id, 0x1234_5678_9ABC_DEF0);
    }

    #[test]
    fn test_keepalive_round_trip_varint() {
        let pkt = CKeepAlive { id: 42 };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_8);
        assert_eq!(decoded.id, 42);
    }

    #[test]
    fn test_keepalive_round_trip_i32() {
        let pkt = CKeepAlive { id: 12345 };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_7_2);
        assert_eq!(decoded.id, 12345);
    }

    #[test]
    fn test_keepalive_serverbound_matches_clientbound() {
        let client = CKeepAlive { id: 99 };
        let mut buf = Vec::new();
        client.encode(&mut buf, ProtocolVersion::V1_21).unwrap();

        let server = SKeepAlive::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(server.id, 99);
    }
}
