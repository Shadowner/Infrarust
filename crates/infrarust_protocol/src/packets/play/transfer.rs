use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};

define_twin_packets! {
    states: {
        #[derive(PartialEq, Eq)]
        CTransfer: Play / Clientbound = ids![
            V1_20_5 => 0x73,
            V1_21_2 => 0x7A,
            V1_21_9 => 0x7F,
            V26_1   => 0x81,
            V26_3   => 0x84,
        ],
        #[derive(PartialEq, Eq)]
        CConfigTransfer: Config / Clientbound = ids![
            V1_20_5 => 0x0B,
            V26_3   => 0x0C,
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

    #[test]
    fn a_transfer_is_a_host_then_a_varint_port() {
        let expected = [&[16][..], b"play.example.com", &[0xdd, 0xc7, 0x01]].concat();
        let host = "play.example.com".to_string();
        for version in [ProtocolVersion::V1_20_5, ProtocolVersion::V1_21_11] {
            let mut play = Vec::new();
            CTransfer {
                host: host.clone(),
                port: 25565,
            }
            .encode(&mut play, version)
            .unwrap();
            assert_eq!(play, expected);
            let mut config = Vec::new();
            CConfigTransfer {
                host: host.clone(),
                port: 25565,
            }
            .encode(&mut config, version)
            .unwrap();
            assert_eq!(config, expected);
        }
    }

    #[test]
    fn the_highest_port_takes_three_bytes() {
        let mut buf = Vec::new();
        CTransfer {
            host: "h".into(),
            port: 65535,
        }
        .encode(&mut buf, ProtocolVersion::V1_21)
        .unwrap();
        assert_eq!(buf, [1, b'h', 0xff, 0xff, 0x03]);
    }
}
