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
        w.write_var_int(&VarInt(id as i32))?;
    } else {
        w.write_i32_be(id as i32)?;
    }
    Ok(())
}

define_twin_packets! {
    clientbound: CKeepAlive,
    serverbound: SKeepAlive,
    state: ConnectionState::Play,
    clientbound_ids: ids![
        V1_7_2  => 0x00,
        V1_9    => 0x1F,
        V1_13   => 0x21,
        V1_14   => 0x20,
        V1_15   => 0x21,
        V1_16   => 0x20,
        V1_16_2 => 0x1F,
        V1_17   => 0x21,
        V1_19   => 0x1E,
        V1_19_1 => 0x20,
        V1_19_3 => 0x1F,
        V1_19_4 => 0x23,
        V1_20_2 => 0x24,
        V1_20_5 => 0x26,
        V1_21_2 => 0x27,
        V1_21_5 => 0x26,
        V1_21_9 => 0x2B,
        V26_1   => 0x2C,
    ],
    serverbound_ids: ids![
        V1_7_2  => 0x00,
        V1_9    => 0x0B,
        V1_12   => 0x0C,
        V1_12_1 => 0x0B,
        V1_13   => 0x0E,
        V1_14   => 0x0F,
        V1_16   => 0x10,
        V1_17   => 0x0F,
        V1_19   => 0x11,
        V1_19_1 => 0x12,
        V1_19_3 => 0x11,
        V1_19_4 => 0x12,
        V1_20_2 => 0x14,
        V1_20_3 => 0x15,
        V1_20_5 => 0x18,
        V1_21_2 => 0x1A,
        V1_21_6 => 0x1B,
        V26_1   => 0x1C,
    ],
    encode_only: false,
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
