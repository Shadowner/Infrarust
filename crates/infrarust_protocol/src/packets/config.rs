use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::Packet;

/// A known data pack entry used in `CKnownPacks` / `SKnownPacks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownPack {
    pub namespace: String,
    pub id: String,
    pub version: String,
}

/// Finish configuration packet (Clientbound).
///
/// Empty signal sent by the server to indicate the end of configuration phase.
#[derive(Debug, Clone)]
pub struct CFinishConfig;

impl Packet for CFinishConfig {
    const NAME: &'static str = "CFinishConfig";

    fn state() -> ConnectionState {
        ConnectionState::Config
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

/// Acknowledge finish configuration packet (Serverbound).
///
/// Empty confirmation from the client that it's ready to transition to Play.
#[derive(Debug, Clone)]
pub struct SAcknowledgeFinishConfig;

impl Packet for SAcknowledgeFinishConfig {
    const NAME: &'static str = "SAcknowledgeFinishConfig";

    fn state() -> ConnectionState {
        ConnectionState::Config
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

/// Registry data packet (Clientbound).
///
/// Contains registry synchronization data (NBT). The proxy stores this as
/// opaque bytes and forwards it without parsing.
#[derive(Debug, Clone)]
pub struct CRegistryData {
    pub data: Vec<u8>,
}

impl Packet for CRegistryData {
    const NAME: &'static str = "CRegistryData";

    fn state() -> ConnectionState {
        ConnectionState::Config
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let data = r.read_remaining()?;
        Ok(Self { data })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_all(&self.data)?;
        Ok(())
    }
}

define_twin_packets! {
    clientbound: CKnownPacks,
    serverbound: SKnownPacks,
    state: ConnectionState::Config,
    fields: {
        pub packs: Vec<KnownPack>,
    },
    decode(r, _version): {
        let count = r.read_var_int()?.0;
        if count < 0 {
            return Err(crate::error::ProtocolError::invalid(
                "negative pack count",
            ));
        }
        let count = count as usize;
        let mut packs = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            packs.push(KnownPack {
                namespace: r.read_string()?,
                id: r.read_string()?,
                version: r.read_string()?,
            });
        }
        Ok(Self { packs })
    },
    encode(self, w, _version): {
        w.write_var_int(&VarInt(self.packs.len() as i32))?;
        for pack in &self.packs {
            w.write_string(&pack.namespace)?;
            w.write_string(&pack.id)?;
            w.write_string(&pack.version)?;
        }
        Ok(())
    },
}

define_twin_packets! {
    clientbound: CConfigPluginMessage,
    serverbound: SConfigPluginMessage,
    state: ConnectionState::Config,
    fields: {
        pub channel: String,
        pub data: Vec<u8>,
    },
    decode(r, _version): {
        let channel = r.read_string()?;
        let data = r.read_remaining()?;
        Ok(Self { channel, data })
    },
    encode(self, w, _version): {
        w.write_string(&self.channel)?;
        w.write_all(&self.data)?;
        Ok(())
    },
}

/// Disconnect packet in config state (Clientbound).
///
/// The reason is stored as opaque bytes because its format varies:
/// - 1.20.2: JSON string
/// - 1.20.3+: NBT compound
///
/// The proxy forwards it without parsing.
#[derive(Debug, Clone)]
pub struct CConfigDisconnect {
    pub reason: Vec<u8>,
}

impl Packet for CConfigDisconnect {
    const NAME: &'static str = "CConfigDisconnect";

    fn state() -> ConnectionState {
        ConnectionState::Config
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let reason = r.read_remaining()?;
        Ok(Self { reason })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_all(&self.reason)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::registry::build_default_registry;

    fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        P::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_finish_config_round_trip() {
        let pkt = CFinishConfig;
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_20_2).unwrap();
        assert!(buf.is_empty());
        CFinishConfig::decode(&mut buf.as_slice(), ProtocolVersion::V1_20_2).unwrap();
    }

    #[test]
    fn test_acknowledge_finish_config_round_trip() {
        let pkt = SAcknowledgeFinishConfig;
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_20_2).unwrap();
        assert!(buf.is_empty());
        SAcknowledgeFinishConfig::decode(&mut buf.as_slice(), ProtocolVersion::V1_20_2).unwrap();
    }

    #[test]
    fn test_registry_data_opaque_payload() {
        let data = vec![0x0A, 0x00, 0x00, 0xFF, 0xAB, 0xCD]; // arbitrary bytes
        let pkt = CRegistryData { data: data.clone() };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_2);
        assert_eq!(decoded.data, data);
    }

    #[test]
    fn test_known_packs_multiple() {
        let pkt = CKnownPacks {
            packs: vec![
                KnownPack {
                    namespace: "minecraft".to_string(),
                    id: "core".to_string(),
                    version: "1.21".to_string(),
                },
                KnownPack {
                    namespace: "custom".to_string(),
                    id: "mypack".to_string(),
                    version: "2.0".to_string(),
                },
            ],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_5);
        assert_eq!(decoded.packs.len(), 2);
        assert_eq!(decoded.packs[0].namespace, "minecraft");
        assert_eq!(decoded.packs[0].id, "core");
        assert_eq!(decoded.packs[1].namespace, "custom");
        assert_eq!(decoded.packs[1].version, "2.0");
    }

    #[test]
    fn test_known_packs_symmetry() {
        let packs = vec![KnownPack {
            namespace: "minecraft".to_string(),
            id: "core".to_string(),
            version: "1.21".to_string(),
        }];

        let c_pkt = CKnownPacks {
            packs: packs.clone(),
        };
        let s_pkt = SKnownPacks {
            packs: packs.clone(),
        };

        let mut c_buf = Vec::new();
        c_pkt.encode(&mut c_buf, ProtocolVersion::V1_20_5).unwrap();
        let mut s_buf = Vec::new();
        s_pkt.encode(&mut s_buf, ProtocolVersion::V1_20_5).unwrap();

        // Same wire format
        assert_eq!(c_buf, s_buf);

        // Cross-decode
        let decoded_s =
            SKnownPacks::decode(&mut c_buf.as_slice(), ProtocolVersion::V1_20_5).unwrap();
        assert_eq!(decoded_s.packs, packs);
    }

    #[test]
    fn test_config_plugin_message_round_trip() {
        let pkt = CConfigPluginMessage {
            channel: "minecraft:brand".to_string(),
            data: vec![0x07, b'I', b'n', b'f', b'r', b'a', b'r', b'u'],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_2);
        assert_eq!(decoded.channel, "minecraft:brand");
        assert_eq!(decoded.data, pkt.data);

        let s_pkt = SConfigPluginMessage {
            channel: "minecraft:brand".to_string(),
            data: vec![0x05, b'T', b'e', b's', b't', b'!'],
        };
        let decoded_s = round_trip(&s_pkt, ProtocolVersion::V1_20_2);
        assert_eq!(decoded_s.channel, "minecraft:brand");
        assert_eq!(decoded_s.data, s_pkt.data);
    }

    #[test]
    fn test_config_disconnect_round_trip() {
        let reason = b"{\"text\":\"Server is restarting\"}".to_vec();
        let pkt = CConfigDisconnect {
            reason: reason.clone(),
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_20_2);
        assert_eq!(decoded.reason, reason);
    }

    #[test]
    fn test_no_config_packets_before_1_20_2() {
        let registry = build_default_registry();

        for version in [ProtocolVersion::V1_19, ProtocolVersion::V1_20] {
            // No config packets should exist for pre-1.20.2 versions
            assert!(
                registry
                    .get_packet_id::<CFinishConfig>(
                        ConnectionState::Config,
                        Direction::Clientbound,
                        version,
                    )
                    .is_none(),
                "CFinishConfig should NOT be registered for {version}"
            );
            assert!(
                registry
                    .get_packet_id::<SAcknowledgeFinishConfig>(
                        ConnectionState::Config,
                        Direction::Serverbound,
                        version,
                    )
                    .is_none(),
                "SAcknowledgeFinishConfig should NOT be registered for {version}"
            );
            assert!(
                registry
                    .get_packet_id::<CRegistryData>(
                        ConnectionState::Config,
                        Direction::Clientbound,
                        version,
                    )
                    .is_none(),
                "CRegistryData should NOT be registered for {version}"
            );
        }
    }
}
