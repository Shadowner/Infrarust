//! Version-aware packet registry that maps packet IDs to decode functions.
//!
//! Supports multiple protocol versions and connection states, providing O(1) lookup
//! for both decoding (packet ID to decoder) and encoding (type to packet ID).

pub mod builder;

pub use builder::{PacketMapping, PacketRegistration, build_default_registry};

use crate::error::ProtocolResult;
use crate::io::PacketFrame;
use crate::packets::{ErasedPacket, Packet};
use crate::version::{ConnectionState, Direction, ProtocolVersion};
use bytes::Bytes;
use std::any::TypeId;
use std::collections::HashMap;

/// Type-erased decoder function stored in the registry.
///
/// Takes a byte slice (payload after the `packet_id`) and the version,
/// returns a boxed packet.
type DecoderFn = fn(&mut &[u8], ProtocolVersion) -> ProtocolResult<Box<dyn ErasedPacket>>;

/// Result of decoding a [`PacketFrame`] through the registry.
///
/// The proxy inspects this result to decide what to do:
/// - `Typed` — the packet is known, can be inspected/modified
/// - `Opaque` — unknown packet, forward the bytes as-is (zero-copy)
#[derive(Debug)]
pub enum DecodedPacket {
    /// Known packet, deserialized into a typed struct.
    Typed {
        /// The packet ID.
        id: i32,
        /// The decoded packet, downcastable via `as_any()`.
        packet: Box<dyn ErasedPacket>,
    },
    /// Unknown packet or no registered parser. Forward as-is.
    Opaque {
        /// The packet ID.
        id: i32,
        /// The raw payload.
        payload: Bytes,
    },
}

/// Registry for a specific (state, direction, version) combination.
/// Contains two O(1) lookups:
/// - decode: `packet_id` → `DecoderFn`
/// - encode: `TypeId` (of the Packet struct) → `packet_id`
#[derive(Default)]
struct VersionRegistry {
    id_to_decoder: HashMap<i32, DecoderFn>,
    type_to_id: HashMap<TypeId, i32>,
}

/// Complete packet registry, immutable after construction.
///
/// Contains a [`VersionRegistry`] per (state, direction, version) combination.
/// Lookup is O(1): two nested `HashMaps`.
///
/// Built via [`PacketRegistration`] builders then frozen behind an `Arc`.
///
/// Pattern transposed from Velocity's `StateRegistry` (Java).
pub struct PacketRegistry {
    registries: HashMap<(ConnectionState, Direction, ProtocolVersion), VersionRegistry>,
}

impl PacketRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            registries: HashMap::new(),
        }
    }

    /// Attempts to decode a [`PacketFrame`] using the registry.
    ///
    /// 1. Looks up the decoder for (state, direction, version, frame.id)
    /// 2. If found → decodes the payload → [`DecodedPacket::Typed`]
    /// 3. If not found → [`DecodedPacket::Opaque`] (zero-copy, no error)
    ///
    /// Returns an error only if a decoder is found but fails
    /// (corrupted data). Missing decoder is NOT an error.
    ///
    /// # Example
    ///
    /// ```
    /// use infrarust_protocol::{
    ///     build_default_registry, DecodedPacket, PacketFrame, Packet,
    ///     SHandshake, VarInt,
    /// };
    /// use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
    /// use bytes::Bytes;
    ///
    /// let registry = build_default_registry();
    ///
    /// // Encode a handshake payload
    /// let handshake = SHandshake {
    ///     protocol_version: VarInt(769),
    ///     server_address: "localhost".to_string(),
    ///     server_port: 25565,
    ///     next_state: ConnectionState::Login,
    /// };
    /// let mut payload = Vec::new();
    /// handshake.encode(&mut payload, ProtocolVersion::V1_21).unwrap();
    ///
    /// // Wrap in a frame (packet id 0x00 for handshake)
    /// let frame = PacketFrame { id: 0x00, payload: Bytes::from(payload) };
    ///
    /// // Decode via the registry → Typed
    /// let decoded = registry.decode_frame(
    ///     &frame,
    ///     ConnectionState::Handshake,
    ///     Direction::Serverbound,
    ///     ProtocolVersion::V1_21,
    /// ).unwrap();
    /// assert!(matches!(decoded, DecodedPacket::Typed { .. }));
    ///
    /// // Unknown packet id → Opaque
    /// let unknown = PacketFrame { id: 0xFF, payload: Bytes::new() };
    /// let decoded = registry.decode_frame(
    ///     &unknown,
    ///     ConnectionState::Handshake,
    ///     Direction::Serverbound,
    ///     ProtocolVersion::V1_21,
    /// ).unwrap();
    /// assert!(matches!(decoded, DecodedPacket::Opaque { .. }));
    /// ```
    ///
    /// # Errors
    /// Returns an error only if a registered decoder fails (corrupted data).
    /// Missing decoder is not an error and returns `DecodedPacket::Opaque`.
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

    /// Gets the `packet_id` for encoding a given typed packet.
    ///
    /// Returns `None` if the packet is not registered for this
    /// (state, direction, version) combination.
    pub fn get_packet_id<P: Packet + 'static>(
        &self,
        state: ConnectionState,
        direction: Direction,
        version: ProtocolVersion,
    ) -> Option<i32> {
        let key = (state, direction, version);
        self.registries
            .get(&key)
            .and_then(|ver_reg| ver_reg.type_to_id.get(&TypeId::of::<P>()).copied())
    }

    /// Checks if a decoder exists for a given `packet_id`.
    ///
    /// Useful for debug and tests.
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

    /// Inserts a type-to-id mapping for encoding.
    /// Used internally by the builder.
    pub(crate) fn insert_type_mapping(
        &mut self,
        key: (ConnectionState, Direction, ProtocolVersion),
        type_id: TypeId,
        packet_id: i32,
    ) {
        let ver_reg = self.registries.entry(key).or_default();
        ver_reg.type_to_id.insert(type_id, packet_id);
    }

    /// Inserts a decoder function for a `packet_id`.
    /// Used internally by the builder.
    pub(crate) fn insert_decoder(
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
    use crate::packets::SHandshake;
    use std::sync::Arc;

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
        let frame = PacketFrame {
            id: 0x00,
            payload: bytes::Bytes::from(payload),
        };

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
        let frame = PacketFrame {
            id: 0xFF,
            payload: bytes::Bytes::from(payload.clone()),
        };

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
        // Register only for V1_9+, then try V1_8-style lookup on a fresh registry
        let mut registry = PacketRegistry::new();
        PacketRegistration::<SHandshake>::new(ConnectionState::Handshake, Direction::Serverbound)
            .map(0x00, ProtocolVersion::V1_9, false)
            .register(&mut registry);

        let payload = make_handshake_payload(47, "mc.example.com", 25565, 2);
        let frame = PacketFrame {
            id: 0x00,
            payload: bytes::Bytes::from(payload),
        };

        // V1_8 is not in the registry (only V1_9+)
        let decoded = registry
            .decode_frame(
                &frame,
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_8,
            )
            .unwrap();

        assert!(matches!(decoded, DecodedPacket::Opaque { .. }));
    }

    #[test]
    fn test_versioned_mapping_different_ids() {
        let mut registry = PacketRegistry::new();
        PacketRegistration::<SHandshake>::new(ConnectionState::Handshake, Direction::Serverbound)
            .map(0x14, ProtocolVersion::V1_7_2, false)
            .map(0x01, ProtocolVersion::V1_9, false)
            .register(&mut registry);

        // V1_8 should have id 0x14
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_8,
            ),
            Some(0x14)
        );

        // V1_9 should have id 0x01
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_9,
            ),
            Some(0x01)
        );
    }

    #[test]
    fn test_version_range_filling() {
        let mut registry = PacketRegistry::new();
        PacketRegistration::<SHandshake>::new(ConnectionState::Handshake, Direction::Serverbound)
            .map(0x00, ProtocolVersion::V1_7_2, false)
            .register(&mut registry);

        // Should be found for all supported versions
        for version in [
            ProtocolVersion::V1_7_2,
            ProtocolVersion::V1_8,
            ProtocolVersion::V1_12,
            ProtocolVersion::V1_21,
            ProtocolVersion::V1_21_4,
        ] {
            assert_eq!(
                registry.get_packet_id::<SHandshake>(
                    ConnectionState::Handshake,
                    Direction::Serverbound,
                    version,
                ),
                Some(0x00),
                "expected packet id for version {version}"
            );
        }
    }

    #[test]
    fn test_mapping_range_stops_at_next_mapping() {
        let mut registry = PacketRegistry::new();
        PacketRegistration::<SHandshake>::new(ConnectionState::Handshake, Direction::Serverbound)
            .map(0x14, ProtocolVersion::V1_7_2, false)
            .map(0x01, ProtocolVersion::V1_9, false)
            .register(&mut registry);

        // V1_8 should have 0x14 (first mapping)
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_8,
            ),
            Some(0x14)
        );

        // V1_9 should have 0x01 (second mapping)
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_9,
            ),
            Some(0x01)
        );

        // V1_8 must NOT have decoder for 0x01
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
        PacketRegistration::<SHandshake>::new(ConnectionState::Handshake, Direction::Serverbound)
            .map_range(
                0x0F,
                ProtocolVersion::V1_17,
                ProtocolVersion::V1_18_2,
                false,
            )
            .register(&mut registry);

        // V1_17 → found
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_17,
            ),
            Some(0x0F)
        );

        // V1_18_2 → found (inclusive)
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_18_2,
            ),
            Some(0x0F)
        );

        // V1_19 → not found
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_19,
            ),
            None
        );
    }

    #[test]
    fn test_encode_only_not_in_decoder() {
        let mut registry = PacketRegistry::new();
        PacketRegistration::<SHandshake>::new(ConnectionState::Handshake, Direction::Serverbound)
            .map(0x10, ProtocolVersion::V1_7_2, true)
            .register(&mut registry);

        // has_decoder should be false (encode_only)
        assert!(!registry.has_decoder(
            ConnectionState::Handshake,
            Direction::Serverbound,
            ProtocolVersion::V1_7_2,
            0x10,
        ));

        // but get_packet_id should return Some
        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Handshake,
                Direction::Serverbound,
                ProtocolVersion::V1_7_2,
            ),
            Some(0x10)
        );
    }

    #[test]
    fn test_get_packet_id_returns_none_for_unregistered() {
        let registry = PacketRegistry::new();

        assert_eq!(
            registry.get_packet_id::<SHandshake>(
                ConnectionState::Play,
                Direction::Clientbound,
                ProtocolVersion::V1_21,
            ),
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
        // Truncated payload — VarInt starts but string is missing
        let frame = PacketFrame {
            id: 0x00,
            payload: bytes::Bytes::from_static(&[0xFF, 0x05]),
        };

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

        // Verify it works from the clone
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

        // Clientbound
        assert!(
            registry
                .get_packet_id::<CKeepAlive>(ConnectionState::Play, Direction::Clientbound, v)
                .is_some()
        );
        assert!(
            registry
                .get_packet_id::<CDisconnect>(ConnectionState::Play, Direction::Clientbound, v)
                .is_some()
        );
        assert!(
            registry
                .get_packet_id::<CJoinGame>(ConnectionState::Play, Direction::Clientbound, v)
                .is_some()
        );
        assert!(
            registry
                .get_packet_id::<CRespawn>(ConnectionState::Play, Direction::Clientbound, v)
                .is_some()
        );
        assert!(
            registry
                .get_packet_id::<CPluginMessage>(ConnectionState::Play, Direction::Clientbound, v)
                .is_some()
        );
        assert!(
            registry
                .get_packet_id::<CSystemChatMessage>(
                    ConnectionState::Play,
                    Direction::Clientbound,
                    v
                )
                .is_some()
        );
        assert!(
            registry
                .get_packet_id::<CTransfer>(ConnectionState::Play, Direction::Clientbound, v)
                .is_some()
        );

        // Serverbound
        assert!(
            registry
                .get_packet_id::<SKeepAlive>(ConnectionState::Play, Direction::Serverbound, v)
                .is_some()
        );
        assert!(
            registry
                .get_packet_id::<SPluginMessage>(ConnectionState::Play, Direction::Serverbound, v)
                .is_some()
        );
    }

    #[test]
    fn test_registry_keepalive_different_ids_by_version() {
        use crate::packets::CKeepAlive;

        let registry = build_default_registry();

        let id_v1_8 = registry
            .get_packet_id::<CKeepAlive>(
                ConnectionState::Play,
                Direction::Clientbound,
                ProtocolVersion::V1_8,
            )
            .unwrap();

        let id_v1_21 = registry
            .get_packet_id::<CKeepAlive>(
                ConnectionState::Play,
                Direction::Clientbound,
                ProtocolVersion::V1_21,
            )
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
                .get_packet_id::<CTransfer>(
                    ConnectionState::Play,
                    Direction::Clientbound,
                    ProtocolVersion::V1_20,
                )
                .is_none(),
            "CTransfer should not be registered before V1_20_5"
        );

        assert!(
            registry
                .get_packet_id::<CTransfer>(
                    ConnectionState::Play,
                    Direction::Clientbound,
                    ProtocolVersion::V1_20_5,
                )
                .is_some(),
            "CTransfer should be registered for V1_20_5+"
        );
    }

    #[test]
    #[allow(clippy::similar_names)] // decoder/decoded are semantically distinct
    fn test_end_to_end_play_packet() {
        use crate::io::{PacketDecoder, PacketEncoder};
        use crate::packets::CKeepAlive;

        let registry = build_default_registry();
        let version = ProtocolVersion::V1_21;
        let state = ConnectionState::Play;
        let direction = Direction::Clientbound;

        // Get the packet ID from registry
        let packet_id = registry
            .get_packet_id::<CKeepAlive>(state, direction, version)
            .expect("CKeepAlive should be registered");

        // Encode the packet payload
        let pkt = CKeepAlive {
            id: 0xDEAD_BEEF_CAFE,
        };
        let mut payload = Vec::new();
        pkt.encode(&mut payload, version).unwrap();

        // Frame it with PacketEncoder
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(packet_id, &payload).unwrap();
        let wire_bytes = encoder.take();

        // Decode with PacketDecoder
        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&wire_bytes);
        let frame = decoder
            .try_next_frame()
            .unwrap()
            .expect("should decode a frame");
        assert_eq!(frame.id, packet_id);

        // Decode via registry
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
}
