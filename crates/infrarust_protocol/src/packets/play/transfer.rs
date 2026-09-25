use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        CTransfer: Play / Clientbound = ids![
            V1_20_5 => 0x73,
            V1_21_2 => 0x7A,
            V1_21_9 => 0x7F,
            V26_1   => 0x81,
        ],
        #[derive(PartialEq, Eq)]
        CConfigTransfer: Config / Clientbound = ids![
            V1_20_5 => 0x0B,
        ],
    },
    encode_only: true,
    fields: {
        pub host: String,
        pub port: i32,
    },
    shared_impl: {},
    decode(r, _version): {
        let host = r.read_string()?;
        let port = r.read_var_int()?.0;
        Ok(Self { host, port })
    },
    encode(self, w, _version): {
        w.write_string(&self.host)?;
        w.write_var_int(&VarInt(self.port))?;
        Ok(())
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::Packet;
    use crate::version::ProtocolVersion;

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

    #[test]
    fn test_config_transfer_round_trip() {
        let pkt = CConfigTransfer {
            host: "hub.example.com".to_string(),
            port: 25566,
        };
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        let decoded = CConfigTransfer::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded, pkt);
    }
}
