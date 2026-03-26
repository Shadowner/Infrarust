use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::{ConnectionState, ProtocolVersion};

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

// Wire format varies by version:
// - 1.7.2 - 1.7.6: i32
// - 1.8 - 1.12.1: VarInt
// - 1.12.2+: i64
define_twin_packets! {
    clientbound: CKeepAlive,
    serverbound: SKeepAlive,
    state: ConnectionState::Play,
    fields: {
        pub id: i64,
    },
    decode(r, version): {
        let id = decode_keepalive_id(r, version)?;
        Ok(Self { id })
    },
    encode(self, w, version): {
        encode_keepalive_id(w, self.id, version)
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::Packet;
    use crate::version::ProtocolVersion;

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
