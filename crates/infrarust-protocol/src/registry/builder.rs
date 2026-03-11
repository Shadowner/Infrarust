use std::any::TypeId;
use std::marker::PhantomData;

use crate::packets::Packet;
use crate::registry::PacketRegistry;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

/// A mapping of a packet_id for a range of versions.
///
/// Equivalent of `PacketMapping` in Velocity.
pub struct PacketMapping {
    /// The packet ID for this version range.
    pub id: i32,
    /// The first version (inclusive) where this mapping is valid.
    pub from: ProtocolVersion,
    /// The last version (inclusive) where this mapping is valid.
    /// `None` = valid until the next mapping or the last supported version.
    pub to: Option<ProtocolVersion>,
    /// If true, the packet is encode-only: the proxy can send it
    /// but won't intercept it on reception.
    pub encode_only: bool,
}

/// Fluent builder for registering a packet in the registry.
///
/// # Example
///
/// ```ignore
/// PacketRegistration::<SHandshake>::new(
///     ConnectionState::Handshake,
///     Direction::Serverbound,
/// )
///     .map(0x00, ProtocolVersion::V1_7_2, false)
///     .register(&mut registry);
/// ```
pub struct PacketRegistration<P: Packet> {
    state: ConnectionState,
    direction: Direction,
    mappings: Vec<PacketMapping>,
    _phantom: PhantomData<P>,
}

impl<P: Packet + 'static> PacketRegistration<P> {
    /// Creates a new builder for a packet in a given state and direction.
    pub fn new(state: ConnectionState, direction: Direction) -> Self {
        Self {
            state,
            direction,
            mappings: Vec::new(),
            _phantom: PhantomData,
        }
    }

    /// Adds a mapping: the packet has ID `id` starting from version `from`.
    ///
    /// This mapping is valid from `from` until the version preceding the next
    /// call to `map()`, or until the last supported version if this is the
    /// last mapping.
    ///
    /// `encode_only`: if true, the proxy can encode this packet but won't
    /// decode it (no DecoderFn in id_to_decoder).
    pub fn map(mut self, id: i32, from: ProtocolVersion, encode_only: bool) -> Self {
        self.mappings.push(PacketMapping {
            id,
            from,
            to: None,
            encode_only,
        });
        self
    }

    /// Adds a mapping with an explicit end version.
    ///
    /// Useful for packets that disappear in a newer version.
    pub fn map_range(
        mut self,
        id: i32,
        from: ProtocolVersion,
        to: ProtocolVersion,
        encode_only: bool,
    ) -> Self {
        self.mappings.push(PacketMapping {
            id,
            from,
            to: Some(to),
            encode_only,
        });
        self
    }

    /// Registers the packet in the registry. Consumes the builder.
    ///
    /// Range filling logic (identical to Velocity's StateRegistry.register()):
    ///
    /// For each mapping\[i\]:
    ///   - from = mapping\[i\].from
    ///   - to = mapping\[i\].to if explicit,
    ///     else mapping\[i+1\].from (exclusive — stop before it)
    ///     else last version in SUPPORTED (if last mapping)
    ///
    /// For each version in \[from, to\]:
    ///   - Always: insert into type_to_id (TypeId → packet_id) for encoding
    ///   - If !encode_only: insert into id_to_decoder (packet_id → DecoderFn) for decoding
    pub fn register(self, registry: &mut PacketRegistry) {
        let last_supported = match ProtocolVersion::SUPPORTED.last() {
            Some(v) => *v,
            None => return,
        };

        let decoder_fn: super::DecoderFn = |r, v| Ok(Box::new(P::decode(r, v)?));

        let type_id = TypeId::of::<P>();

        for (i, mapping) in self.mappings.iter().enumerate() {
            let from = mapping.from;

            let (to, inclusive) = if let Some(explicit_to) = mapping.to {
                (explicit_to, true)
            } else if let Some(next_mapping) = self.mappings.get(i + 1) {
                (next_mapping.from, false) // exclusive
            } else {
                (last_supported, true)
            };

            for version in ProtocolVersion::range(from, to) {
                if !inclusive && version == to {
                    continue;
                }

                let key = (self.state, self.direction, version);

                registry.insert_type_mapping(key, type_id, mapping.id);

                if !mapping.encode_only {
                    registry.insert_decoder(key, mapping.id, decoder_fn);
                }
            }
        }
    }
}

/// Builds the default registry with all packets known to the proxy.
///
/// Packet IDs sourced from Velocity's StateRegistry.java.
pub fn build_default_registry() -> PacketRegistry {
    let mut registry = PacketRegistry::new();

    // ── Handshake ───────────────────────────────────────────────────
    PacketRegistration::<crate::packets::SHandshake>::new(
        ConnectionState::Handshake,
        Direction::Serverbound,
    )
    .map(0x00, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    // ── Status ──────────────────────────────────────────────────────
    PacketRegistration::<crate::packets::SStatusRequest>::new(
        ConnectionState::Status,
        Direction::Serverbound,
    )
    .map(0x00, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::SPingRequest>::new(
        ConnectionState::Status,
        Direction::Serverbound,
    )
    .map(0x01, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CStatusResponse>::new(
        ConnectionState::Status,
        Direction::Clientbound,
    )
    .map(0x00, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CPingResponse>::new(
        ConnectionState::Status,
        Direction::Clientbound,
    )
    .map(0x01, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    // ── Login (Serverbound) ─────────────────────────────────────────
    PacketRegistration::<crate::packets::SLoginStart>::new(
        ConnectionState::Login,
        Direction::Serverbound,
    )
    .map(0x00, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::SEncryptionResponse>::new(
        ConnectionState::Login,
        Direction::Serverbound,
    )
    .map(0x01, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::SLoginPluginResponse>::new(
        ConnectionState::Login,
        Direction::Serverbound,
    )
    .map(0x02, ProtocolVersion::V1_13, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::SLoginAcknowledged>::new(
        ConnectionState::Login,
        Direction::Serverbound,
    )
    .map(0x03, ProtocolVersion::V1_20_2, false)
    .register(&mut registry);

    // ── Login (Clientbound) ─────────────────────────────────────────
    PacketRegistration::<crate::packets::CLoginDisconnect>::new(
        ConnectionState::Login,
        Direction::Clientbound,
    )
    .map(0x00, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CEncryptionRequest>::new(
        ConnectionState::Login,
        Direction::Clientbound,
    )
    .map(0x01, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CLoginSuccess>::new(
        ConnectionState::Login,
        Direction::Clientbound,
    )
    .map(0x02, ProtocolVersion::V1_7_2, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CSetCompression>::new(
        ConnectionState::Login,
        Direction::Clientbound,
    )
    .map(0x03, ProtocolVersion::V1_8, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CLoginPluginRequest>::new(
        ConnectionState::Login,
        Direction::Clientbound,
    )
    .map(0x04, ProtocolVersion::V1_13, false)
    .register(&mut registry);

    // ── Config (Serverbound) ────────────────────────────────────────
    PacketRegistration::<crate::packets::SConfigPluginMessage>::new(
        ConnectionState::Config,
        Direction::Serverbound,
    )
    .map(0x01, ProtocolVersion::V1_20_2, false)
    .map(0x02, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::SAcknowledgeFinishConfig>::new(
        ConnectionState::Config,
        Direction::Serverbound,
    )
    .map(0x02, ProtocolVersion::V1_20_2, false)
    .map(0x03, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::SKnownPacks>::new(
        ConnectionState::Config,
        Direction::Serverbound,
    )
    .map(0x07, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    // ── Config (Clientbound) ────────────────────────────────────────
    PacketRegistration::<crate::packets::CConfigPluginMessage>::new(
        ConnectionState::Config,
        Direction::Clientbound,
    )
    .map(0x00, ProtocolVersion::V1_20_2, false)
    .map(0x01, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CConfigDisconnect>::new(
        ConnectionState::Config,
        Direction::Clientbound,
    )
    .map(0x01, ProtocolVersion::V1_20_2, false)
    .map(0x02, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CFinishConfig>::new(
        ConnectionState::Config,
        Direction::Clientbound,
    )
    .map(0x02, ProtocolVersion::V1_20_2, false)
    .map(0x03, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CRegistryData>::new(
        ConnectionState::Config,
        Direction::Clientbound,
    )
    .map(0x05, ProtocolVersion::V1_20_2, false)
    .map(0x07, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    PacketRegistration::<crate::packets::CKnownPacks>::new(
        ConnectionState::Config,
        Direction::Clientbound,
    )
    .map(0x0E, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    // ── Play (Clientbound) ──────────────────────────────────────────

    // KeepAlive Clientbound
    PacketRegistration::<crate::packets::CKeepAlive>::new(
        ConnectionState::Play,
        Direction::Clientbound,
    )
    .map(0x00, ProtocolVersion::V1_7_2, false)
    .map(0x1F, ProtocolVersion::V1_9, false)
    .map(0x21, ProtocolVersion::V1_13, false)
    .map(0x20, ProtocolVersion::V1_14, false)
    .map(0x21, ProtocolVersion::V1_15, false)
    .map(0x20, ProtocolVersion::V1_16, false)
    .map(0x1F, ProtocolVersion::V1_16_2, false)
    .map(0x21, ProtocolVersion::V1_17, false)
    .map(0x1E, ProtocolVersion::V1_19, false)
    .map(0x20, ProtocolVersion::V1_19_1, false)
    .map(0x1F, ProtocolVersion::V1_19_3, false)
    .map(0x23, ProtocolVersion::V1_19_4, false)
    .map(0x24, ProtocolVersion::V1_20_2, false)
    .map(0x26, ProtocolVersion::V1_20_5, false)
    .map(0x27, ProtocolVersion::V1_21_2, false)
    .register(&mut registry);

    // Disconnect Clientbound (Play)
    PacketRegistration::<crate::packets::CDisconnect>::new(
        ConnectionState::Play,
        Direction::Clientbound,
    )
    .map(0x40, ProtocolVersion::V1_7_2, false)
    .map(0x1A, ProtocolVersion::V1_9, false)
    .map(0x1B, ProtocolVersion::V1_13, false)
    .map(0x1A, ProtocolVersion::V1_14, false)
    .map(0x1B, ProtocolVersion::V1_15, false)
    .map(0x1A, ProtocolVersion::V1_16, false)
    .map(0x19, ProtocolVersion::V1_16_2, false)
    .map(0x1A, ProtocolVersion::V1_17, false)
    .map(0x17, ProtocolVersion::V1_19, false)
    .map(0x19, ProtocolVersion::V1_19_1, false)
    .map(0x17, ProtocolVersion::V1_19_3, false)
    .map(0x1A, ProtocolVersion::V1_19_4, false)
    .map(0x1B, ProtocolVersion::V1_20_2, false)
    .map(0x1D, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    // JoinGame Clientbound
    PacketRegistration::<crate::packets::CJoinGame>::new(
        ConnectionState::Play,
        Direction::Clientbound,
    )
    .map(0x01, ProtocolVersion::V1_7_2, false)
    .map(0x23, ProtocolVersion::V1_9, false)
    .map(0x25, ProtocolVersion::V1_13, false)
    .map(0x25, ProtocolVersion::V1_14, false)
    .map(0x26, ProtocolVersion::V1_15, false)
    .map(0x25, ProtocolVersion::V1_16, false)
    .map(0x24, ProtocolVersion::V1_16_2, false)
    .map(0x26, ProtocolVersion::V1_17, false)
    .map(0x23, ProtocolVersion::V1_19, false)
    .map(0x25, ProtocolVersion::V1_19_1, false)
    .map(0x24, ProtocolVersion::V1_19_3, false)
    .map(0x28, ProtocolVersion::V1_19_4, false)
    .map(0x29, ProtocolVersion::V1_20_2, false)
    .map(0x2B, ProtocolVersion::V1_20_5, false)
    .map(0x2C, ProtocolVersion::V1_21_2, false)
    .register(&mut registry);

    // Respawn Clientbound
    PacketRegistration::<crate::packets::CRespawn>::new(
        ConnectionState::Play,
        Direction::Clientbound,
    )
    .map(0x07, ProtocolVersion::V1_7_2, false)
    .map(0x33, ProtocolVersion::V1_9, false)
    .map(0x34, ProtocolVersion::V1_12, false)
    .map(0x35, ProtocolVersion::V1_12_1, false)
    .map(0x38, ProtocolVersion::V1_13, false)
    .map(0x3A, ProtocolVersion::V1_14, false)
    .map(0x3B, ProtocolVersion::V1_15, false)
    .map(0x3A, ProtocolVersion::V1_16, false)
    .map(0x39, ProtocolVersion::V1_16_2, false)
    .map(0x3D, ProtocolVersion::V1_17, false)
    .map(0x3B, ProtocolVersion::V1_19, false)
    .map(0x3E, ProtocolVersion::V1_19_1, false)
    .map(0x3D, ProtocolVersion::V1_19_3, false)
    .map(0x41, ProtocolVersion::V1_19_4, false)
    .map(0x43, ProtocolVersion::V1_20_2, false)
    .map(0x45, ProtocolVersion::V1_20_3, false)
    .map(0x47, ProtocolVersion::V1_20_5, false)
    .map(0x4C, ProtocolVersion::V1_21_2, false)
    .register(&mut registry);

    // PluginMessage Clientbound
    PacketRegistration::<crate::packets::CPluginMessage>::new(
        ConnectionState::Play,
        Direction::Clientbound,
    )
    .map(0x3F, ProtocolVersion::V1_7_2, false)
    .map(0x18, ProtocolVersion::V1_9, false)
    .map(0x19, ProtocolVersion::V1_13, false)
    .map(0x18, ProtocolVersion::V1_14, false)
    .map(0x19, ProtocolVersion::V1_15, false)
    .map(0x18, ProtocolVersion::V1_16, false)
    .map(0x17, ProtocolVersion::V1_16_2, false)
    .map(0x18, ProtocolVersion::V1_17, false)
    .map(0x15, ProtocolVersion::V1_19, false)
    .map(0x16, ProtocolVersion::V1_19_1, false)
    .map(0x15, ProtocolVersion::V1_19_3, false)
    .map(0x17, ProtocolVersion::V1_19_4, false)
    .map(0x18, ProtocolVersion::V1_20_2, false)
    .map(0x19, ProtocolVersion::V1_20_5, false)
    .register(&mut registry);

    // SystemChatMessage Clientbound (1.19+)
    PacketRegistration::<crate::packets::CSystemChatMessage>::new(
        ConnectionState::Play,
        Direction::Clientbound,
    )
    .map(0x5F, ProtocolVersion::V1_19, false)
    .map(0x62, ProtocolVersion::V1_19_1, false)
    .map(0x60, ProtocolVersion::V1_19_3, false)
    .map(0x64, ProtocolVersion::V1_19_4, false)
    .map(0x67, ProtocolVersion::V1_20_2, false)
    .map(0x69, ProtocolVersion::V1_20_3, false)
    .map(0x6C, ProtocolVersion::V1_20_5, false)
    .map(0x73, ProtocolVersion::V1_21_2, false)
    .register(&mut registry);

    // Transfer Clientbound (1.20.5+)
    PacketRegistration::<crate::packets::CTransfer>::new(
        ConnectionState::Play,
        Direction::Clientbound,
    )
    .map(0x73, ProtocolVersion::V1_20_5, false)
    .map(0x7A, ProtocolVersion::V1_21_2, false)
    .register(&mut registry);

    // ── Play (Serverbound) ────────────────────────────────────────

    // KeepAlive Serverbound
    PacketRegistration::<crate::packets::SKeepAlive>::new(
        ConnectionState::Play,
        Direction::Serverbound,
    )
    .map(0x00, ProtocolVersion::V1_7_2, false)
    .map(0x0B, ProtocolVersion::V1_9, false)
    .map(0x0C, ProtocolVersion::V1_12, false)
    .map(0x0B, ProtocolVersion::V1_12_1, false)
    .map(0x0E, ProtocolVersion::V1_13, false)
    .map(0x0F, ProtocolVersion::V1_14, false)
    .map(0x10, ProtocolVersion::V1_16, false)
    .map(0x0F, ProtocolVersion::V1_17, false)
    .map(0x11, ProtocolVersion::V1_19, false)
    .map(0x12, ProtocolVersion::V1_19_1, false)
    .map(0x11, ProtocolVersion::V1_19_3, false)
    .map(0x12, ProtocolVersion::V1_19_4, false)
    .map(0x14, ProtocolVersion::V1_20_2, false)
    .map(0x15, ProtocolVersion::V1_20_3, false)
    .map(0x18, ProtocolVersion::V1_20_5, false)
    .map(0x1A, ProtocolVersion::V1_21_2, false)
    .register(&mut registry);

    // PluginMessage Serverbound
    PacketRegistration::<crate::packets::SPluginMessage>::new(
        ConnectionState::Play,
        Direction::Serverbound,
    )
    .map(0x17, ProtocolVersion::V1_7_2, false)
    .map(0x09, ProtocolVersion::V1_9, false)
    .map(0x0A, ProtocolVersion::V1_12, false)
    .map(0x09, ProtocolVersion::V1_12_1, false)
    .map(0x0A, ProtocolVersion::V1_13, false)
    .map(0x0B, ProtocolVersion::V1_14, false)
    .map(0x0A, ProtocolVersion::V1_17, false)
    .map(0x0C, ProtocolVersion::V1_19, false)
    .map(0x0D, ProtocolVersion::V1_19_1, false)
    .map(0x0C, ProtocolVersion::V1_19_3, false)
    .map(0x0D, ProtocolVersion::V1_19_4, false)
    .map(0x0F, ProtocolVersion::V1_20_2, false)
    .map(0x10, ProtocolVersion::V1_20_3, false)
    .map(0x12, ProtocolVersion::V1_20_5, false)
    .map(0x14, ProtocolVersion::V1_21_2, false)
    .register(&mut registry);

    registry
}
