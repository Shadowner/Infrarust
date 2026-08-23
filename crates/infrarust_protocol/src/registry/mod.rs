pub mod default;

pub use crate::packets::PacketMapping;
pub use default::{DEFAULT_PACKETS, PacketDescriptor, build_default_registry};

use crate::error::ProtocolResult;
use crate::io::PacketFrame;
use crate::packets::{ErasedPacket, Packet};
use crate::version::{ConnectionState, Direction, ProtocolVersion};
use bytes::Bytes;
use std::any::TypeId;
use std::collections::HashMap;

type DecoderFn = fn(&mut &[u8], ProtocolVersion) -> ProtocolResult<Box<dyn ErasedPacket>>;

#[derive(Debug)]
pub enum DecodedPacket {
    Typed {
        id: i32,
        packet: Box<dyn ErasedPacket>,
    },
    Opaque {
        id: i32,
        payload: Bytes,
    },
}

#[derive(Default)]
struct VersionRegistry {
    id_to_decoder: HashMap<i32, DecoderFn>,
    type_to_id: HashMap<TypeId, i32>,
}

pub struct PacketRegistry {
    registries: HashMap<(ConnectionState, Direction, ProtocolVersion), VersionRegistry>,
}

impl PacketRegistry {
    pub fn new() -> Self {
        Self {
            registries: HashMap::new(),
        }
    }

    pub fn decode_frame(
        &self,
        frame: &PacketFrame,
        state: ConnectionState,
        direction: Direction,
        version: ProtocolVersion,
    ) -> ProtocolResult<DecodedPacket> {
        let key = (state, direction, version);

        if let Some(ver_reg) = self.registries.get(&key)
            && let Some(decoder) = ver_reg.id_to_decoder.get(&frame.id)
        {
            let mut payload = frame.payload.as_ref();
            let packet = decoder(&mut payload, version)?;
            return Ok(DecodedPacket::Typed {
                id: frame.id,
                packet,
            });
        }

        Ok(DecodedPacket::Opaque {
            id: frame.id,
            payload: frame.payload.clone(),
        })
    }

    pub fn register<P: Packet>(&mut self) {
        debug_assert!(
            P::IDS.windows(2).all(|w| w[0].from < w[1].from),
            "{} mappings must be strictly ascending by `from`",
            P::NAME
        );
        debug_assert!(
            P::IDS
                .windows(2)
                .all(|w| w[0].id != w[1].id || w[0].to.is_some()),
            "{} has consecutive mappings with the same id; drop the redundant one",
            P::NAME
        );

        let Some(&last_supported) = ProtocolVersion::SUPPORTED.last() else {
            return;
        };

        let decoder: DecoderFn = |r, v| Ok(Box::new(P::decode(r, v)?));
        let type_id = TypeId::of::<P>();

        for (i, mapping) in P::IDS.iter().enumerate() {
            let (to, inclusive) = match (mapping.to, P::IDS.get(i + 1)) {
                (Some(explicit_to), _) => (explicit_to, true),
                (None, Some(next)) => (next.from, false),
                (None, None) => (last_supported, true),
            };

            for version in ProtocolVersion::range(mapping.from, to) {
                if !inclusive && version == to {
                    continue;
                }

                let key = (P::STATE, P::DIRECTION, version);
                self.insert_type_mapping(key, type_id, mapping.id);

                if !P::ENCODE_ONLY {
                    self.insert_decoder(key, mapping.id, decoder);
                }
            }
        }
    }

    pub fn get_packet_id<P: Packet>(&self, version: ProtocolVersion) -> Option<i32> {
        self.registries
            .get(&(P::STATE, P::DIRECTION, version))
            .and_then(|ver_reg| ver_reg.type_to_id.get(&TypeId::of::<P>()).copied())
    }

    pub fn has_decoder(
        &self,
        state: ConnectionState,
        direction: Direction,
        version: ProtocolVersion,
        packet_id: i32,
    ) -> bool {
        let key = (state, direction, version);
        self.registries
            .get(&key)
            .is_some_and(|ver_reg| ver_reg.id_to_decoder.contains_key(&packet_id))
    }

    pub(crate) fn insert_type_mapping(
        &mut self,
        key: (ConnectionState, Direction, ProtocolVersion),
        type_id: TypeId,
        packet_id: i32,
    ) {
        let ver_reg = self.registries.entry(key).or_default();
        ver_reg.type_to_id.insert(type_id, packet_id);
    }

    fn insert_decoder(
        &mut self,
        key: (ConnectionState, Direction, ProtocolVersion),
        packet_id: i32,
        decoder: DecoderFn,
    ) {
        let ver_reg = self.registries.entry(key).or_default();
        ver_reg.id_to_decoder.insert(packet_id, decoder);
    }
}

impl Default for PacketRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::codec::{McBufWriteExt, VarInt};
    use crate::packets::{
        CEncryptionRequest, CLoginPluginRequest, CLoginSuccess, SHandshake, SLoginAcknowledged,
        SLoginPluginResponse,
    };
    use std::sync::Arc;

    macro_rules! test_packet {
        ($(#[$meta:meta])* $name:ident, $encode_only:expr, $ids:expr) => {
            $(#[$meta])*
            #[derive(Debug)]
            struct $name;

            impl Packet for $name {
                const NAME: &'static str = stringify!($name);
                const STATE: ConnectionState = ConnectionState::Handshake;
                const DIRECTION: Direction = Direction::Serverbound;
                const IDS: &'static [PacketMapping] = $ids;
                const ENCODE_ONLY: bool = $encode_only;

                fn decode(_r: &mut &[u8], _v: ProtocolVersion) -> ProtocolResult<Self> {
                    Ok(Self)
                }

                fn encode(
                    &self,
                    _w: &mut (impl std::io::Write + ?Sized),
                    _v: ProtocolVersion,
                ) -> ProtocolResult<()> {
                    Ok(())
                }
            }
        };
    }

    test_packet!(LateIds, false, ids![V1_9 => 0x00]);

    test_packet!(SplitIds, false, ids![V1_7_2 => 0x14, V1_9 => 0x01]);

    test_packet!(OpenEndedIds, false, ids![V1_7_2 => 0x00]);

    test_packet!(BoundedIds, false, ids![V1_17 ..= V1_18_2 => 0x0F]);

    test_packet!(EncodeOnlyIds, true, ids![V1_7_2 => 0x10]);

    #[cfg(debug_assertions)]
    test_packet!(DescendingIds, false, ids![V1_9 => 0x01, V1_7_2 => 0x00]);

    fn make_handshake_payload(
        protocol_version: i32,
        address: &str,
        port: u16,
        next_state: i32,
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.write_var_int(&VarInt(protocol_version)).unwrap();
        buf.write_string(address).unwrap();
        buf.write_u16_be(port).unwrap();
        buf.write_var_int(&VarInt(next_state)).unwrap();
        buf
    }

    #[test]
    fn test_decode_registered_packet_returns_typed() {
        let registry = build_default_registry();
        let payload = make_handshake_payload(767, "mc.example.com", 25565, 2);
        let frame = PacketFrame::new(0x00, Bytes::from(payload));

        let decoded = registry
            .decode_frame(
                &frame,
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_21,
            )
            .unwrap();

        match decoded {
            DecodedPacket::Typed { id, packet } => {
                assert_eq!(id, 0x00);
                let hs = packet.as_any().downcast_ref::<SHandshake>().unwrap();
                assert_eq!(hs.protocol_version, VarInt(767));
                assert_eq!(hs.server_address, "mc.example.com");
                assert_eq!(hs.server_port, 25565);
                assert_eq!(hs.next_state, ConnectionState::Login);
            }
            DecodedPacket::Opaque { .. } => panic!("expected Typed"),
        }
    }

    #[test]
    fn test_decode_unknown_id_returns_opaque() {
        let registry = build_default_registry();
        let payload = vec![1, 2, 3, 4];
        let frame = PacketFrame::new(0xFF, Bytes::from(payload.clone()));

        let decoded = registry
            .decode_frame(
                &frame,
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_21,
            )
            .unwrap();

        match decoded {
            DecodedPacket::Opaque { id, payload: p } => {
                assert_eq!(id, 0xFF);
                assert_eq!(p.as_ref(), &payload[..]);
            }
            DecodedPacket::Typed { .. } => panic!("expected Opaque"),
        }
    }

    #[test]
    fn test_decode_unknown_version_returns_opaque() {
        let mut registry = PacketRegistry::new();
        registry.register::<LateIds>();

        let frame = PacketFrame::new(0x00, Bytes::from_static(&[1, 2, 3, 4]));

        let decoded = registry
            .decode_frame(
                &frame,
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_8,
            )
            .unwrap();

        assert!(matches!(decoded, DecodedPacket::Opaque { .. }));

        let decoded = registry
            .decode_frame(
                &frame,
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_9,
            )
            .unwrap();

        assert!(matches!(decoded, DecodedPacket::Typed { .. }));
    }

    #[test]
    fn test_versioned_mapping_different_ids() {
        let mut registry = PacketRegistry::new();
        registry.register::<SplitIds>();

        assert_eq!(
            registry.get_packet_id::<SplitIds>(ProtocolVersion::V1_8),
            Some(0x14)
        );

        assert_eq!(
            registry.get_packet_id::<SplitIds>(ProtocolVersion::V1_9),
            Some(0x01)
        );
    }

    #[test]
    fn test_protocol_26_2_login_mappings() {
        let registry = build_default_registry();
        let version = ProtocolVersion::V26_2;

        assert_eq!(
            registry.get_packet_id::<CEncryptionRequest>(version),
            Some(0x01)
        );
        assert_eq!(registry.get_packet_id::<CLoginSuccess>(version), Some(0x02));
        assert_eq!(
            registry.get_packet_id::<CLoginPluginRequest>(version),
            Some(0x04)
        );
        assert_eq!(
            registry.get_packet_id::<SLoginPluginResponse>(version),
            Some(0x02)
        );
        assert_eq!(
            registry.get_packet_id::<SLoginAcknowledged>(version),
            Some(0x03)
        );
    }

    #[test]
    fn test_version_range_filling() {
        let mut registry = PacketRegistry::new();
        registry.register::<OpenEndedIds>();

        for version in [
            ProtocolVersion::V1_7_2,
            ProtocolVersion::V1_8,
            ProtocolVersion::V1_12,
            ProtocolVersion::V1_21,
            ProtocolVersion::V1_21_4,
        ] {
            assert_eq!(
                registry.get_packet_id::<OpenEndedIds>(version),
                Some(0x00),
                "expected packet id for version {version}"
            );
        }
    }

    #[test]
    fn test_mapping_range_stops_at_next_mapping() {
        let mut registry = PacketRegistry::new();
        registry.register::<SplitIds>();

        assert_eq!(
            registry.get_packet_id::<SplitIds>(ProtocolVersion::V1_8),
            Some(0x14)
        );

        assert_eq!(
            registry.get_packet_id::<SplitIds>(ProtocolVersion::V1_9),
            Some(0x01)
        );

        assert!(!registry.has_decoder(
            ConnectionState::Handshake,
            Direction::Serverbound,
            ProtocolVersion::V1_8,
            0x01,
        ));
    }

    #[test]
    fn test_explicit_to_range() {
        let mut registry = PacketRegistry::new();
        registry.register::<BoundedIds>();

        assert_eq!(
            registry.get_packet_id::<BoundedIds>(ProtocolVersion::V1_17),
            Some(0x0F)
        );

        assert_eq!(
            registry.get_packet_id::<BoundedIds>(ProtocolVersion::V1_18_2),
            Some(0x0F)
        );

        assert_eq!(
            registry.get_packet_id::<BoundedIds>(ProtocolVersion::V1_19),
            None
        );
    }

    #[test]
    fn test_encode_only_not_in_decoder() {
        let mut registry = PacketRegistry::new();
        registry.register::<EncodeOnlyIds>();

        assert!(!registry.has_decoder(
            ConnectionState::Handshake,
            Direction::Serverbound,
            ProtocolVersion::V1_7_2,
            0x10,
        ));

        assert_eq!(
            registry.get_packet_id::<EncodeOnlyIds>(ProtocolVersion::V1_7_2),
            Some(0x10)
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "strictly ascending")]
    fn out_of_order_mapping_panics_in_debug() {
        let mut registry = PacketRegistry::new();
        registry.register::<DescendingIds>();
    }

    #[test]
    fn test_get_packet_id_returns_none_for_unregistered() {
        let registry = PacketRegistry::new();

        assert_eq!(
            registry.get_packet_id::<SHandshake>(ProtocolVersion::V1_21),
            None
        );
    }

    #[test]
    fn test_default_registry_has_handshake() {
        let registry = build_default_registry();

        assert!(registry.has_decoder(
            ConnectionState::Handshake,
            Direction::Serverbound,
            ProtocolVersion::V1_7_2,
            0x00,
        ));

        assert!(registry.has_decoder(
            ConnectionState::Handshake,
            Direction::Serverbound,
            ProtocolVersion::V1_21,
            0x00,
        ));
    }

    #[test]
    fn test_decode_frame_with_corrupted_payload_returns_error() {
        let registry = build_default_registry();
        let frame = PacketFrame::new(0x00, Bytes::from_static(&[0xFF, 0x05]));

        let result = registry.decode_frame(
            &frame,
            ConnectionState::Handshake,
            Direction::Serverbound,
            ProtocolVersion::V1_21,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_registry_can_be_shared_via_arc() {
        let registry = Arc::new(build_default_registry());
        let clone = Arc::clone(&registry);

        assert!(clone.has_decoder(
            ConnectionState::Handshake,
            Direction::Serverbound,
            ProtocolVersion::V1_21,
            0x00,
        ));
    }

    #[test]
    fn test_registry_has_all_play_packets() {
        use crate::packets::{
            CDisconnect, CJoinGame, CKeepAlive, CPluginMessage, CRespawn, CSystemChatMessage,
            CTransfer, SKeepAlive, SPluginMessage,
        };

        let registry = build_default_registry();
        let v = ProtocolVersion::V1_21;

        assert!(registry.get_packet_id::<CKeepAlive>(v).is_some());
        assert!(registry.get_packet_id::<CDisconnect>(v).is_some());
        assert!(registry.get_packet_id::<CJoinGame>(v).is_some());
        assert!(registry.get_packet_id::<CRespawn>(v).is_some());
        assert!(registry.get_packet_id::<CPluginMessage>(v).is_some());
        assert!(registry.get_packet_id::<CSystemChatMessage>(v).is_some());
        assert!(registry.get_packet_id::<CTransfer>(v).is_some());

        assert!(registry.get_packet_id::<SKeepAlive>(v).is_some());
        assert!(registry.get_packet_id::<SPluginMessage>(v).is_some());
    }

    #[test]
    fn test_registry_keepalive_different_ids_by_version() {
        use crate::packets::CKeepAlive;

        let registry = build_default_registry();

        let id_v1_8 = registry
            .get_packet_id::<CKeepAlive>(ProtocolVersion::V1_8)
            .unwrap();

        let id_v1_21 = registry
            .get_packet_id::<CKeepAlive>(ProtocolVersion::V1_21)
            .unwrap();

        assert_ne!(id_v1_8, id_v1_21);
        assert_eq!(id_v1_8, 0x00);
    }

    #[test]
    fn test_transfer_not_registered_before_1_20_5() {
        use crate::packets::CTransfer;

        let registry = build_default_registry();

        assert!(
            registry
                .get_packet_id::<CTransfer>(ProtocolVersion::V1_20)
                .is_none(),
            "CTransfer should not be registered before V1_20_5"
        );

        assert!(
            registry
                .get_packet_id::<CTransfer>(ProtocolVersion::V1_20_5)
                .is_some(),
            "CTransfer should be registered for V1_20_5+"
        );
    }

    #[test]
    #[allow(clippy::similar_names)]
    fn test_end_to_end_play_packet() {
        use crate::io::{PacketDecoder, PacketEncoder};
        use crate::packets::CKeepAlive;

        let registry = build_default_registry();
        let version = ProtocolVersion::V1_21;
        let state = ConnectionState::Play;
        let direction = Direction::Clientbound;

        let packet_id = registry
            .get_packet_id::<CKeepAlive>(version)
            .expect("CKeepAlive should be registered");

        let pkt = CKeepAlive {
            id: 0xDEAD_BEEF_CAFE,
        };
        let mut payload = Vec::new();
        pkt.encode(&mut payload, version).unwrap();

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(packet_id, &payload).unwrap();
        let wire_bytes = encoder.take();

        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&wire_bytes);
        let frame = decoder
            .try_next_frame()
            .unwrap()
            .expect("should decode a frame");
        assert_eq!(frame.id, packet_id);

        let decoded = registry
            .decode_frame(&frame, state, direction, version)
            .unwrap();

        match decoded {
            DecodedPacket::Typed { id, packet } => {
                assert_eq!(id, packet_id);
                let keepalive = packet
                    .as_any()
                    .downcast_ref::<CKeepAlive>()
                    .expect("should downcast to CKeepAlive");
                assert_eq!(keepalive.id, 0xDEAD_BEEF_CAFE);
            }
            DecodedPacket::Opaque { .. } => panic!("expected Typed, got Opaque"),
        }
    }

    const ALL_STATES: [ConnectionState; 5] = [
        ConnectionState::Handshake,
        ConnectionState::Status,
        ConnectionState::Login,
        ConnectionState::Config,
        ConnectionState::Play,
    ];

    const ALL_DIRECTIONS: [Direction; 2] = [Direction::Serverbound, Direction::Clientbound];

    fn dump_decode_side(reg: &PacketRegistry, out: &mut Vec<String>) {
        for state in ALL_STATES {
            for direction in ALL_DIRECTIONS {
                for &version in ProtocolVersion::SUPPORTED {
                    for id in 0..=0xFF_i32 {
                        if reg.has_decoder(state, direction, version, id) {
                            out.push(format!(
                                "DEC {state} {direction} {:>3} 0x{id:02X}",
                                version.0
                            ));
                        }
                    }
                }
            }
        }
    }

    fn snapshot(reg: &PacketRegistry) -> String {
        let mut out = Vec::new();
        for descriptor in DEFAULT_PACKETS {
            for &version in ProtocolVersion::SUPPORTED {
                if let Some(id) = (descriptor.packet_id)(reg, version) {
                    out.push(format!(
                        "ENC {} {} {:>3} {} 0x{id:02X}",
                        descriptor.state, descriptor.direction, version.0, descriptor.name
                    ));
                }
            }
        }
        dump_decode_side(reg, &mut out);
        out.sort();
        out.join("\n")
    }

    const SNAPSHOT_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/registry/testdata/registry_snapshot.txt"
    );

    #[test]
    #[ignore = "bless: rewrites the registry snapshot fixture"]
    fn bless_registry_snapshot() {
        std::fs::write(SNAPSHOT_PATH, snapshot(&build_default_registry())).unwrap();
    }

    #[test]
    fn default_registry_matches_snapshot() {
        let expected = include_str!("testdata/registry_snapshot.txt");
        assert_eq!(
            snapshot(&build_default_registry()),
            expected,
            "registry content changed; re-bless with `cargo test -p infrarust_protocol \
             bless_registry_snapshot -- --ignored` and review the fixture diff"
        );
    }
}
