//! Minecraft protocol codec for the Infrarust proxy.
//!
//! This crate implements the Minecraft Java Edition protocol, providing
//! packet framing, encoding/decoding, and version-aware type definitions
//! for the Infrarust reverse proxy.
//!
//! # Architecture
//!
//! - [`error`] — Unified error type for all protocol operations.
//! - [`version`] — Protocol version identifiers, connection states, and packet directions.

pub mod codec;
pub mod crypto;
pub mod error;
pub mod io;
pub mod legacy;
pub mod packets;
pub mod registry;
pub mod version;

pub use codec::{Decode, Encode, McBufReadExt, McBufWriteExt, VarInt, VarLong};
pub use crypto::{DecryptCipher, EncryptCipher};
pub use error::{ProtocolError, ProtocolResult};
pub use io::{PacketDecoder, PacketEncoder, PacketFrame};
pub use legacy::{
    LegacyDetection, LegacyPingRequest, LegacyPingResponse, LegacyPingVariant,
    detect as detect_legacy, parse_legacy_ping,
};
pub use packets::{
    CConfigDisconnect, CConfigPluginMessage, CDisconnect, CEncryptionRequest, CFinishConfig,
    CJoinGame, CKeepAlive, CKnownPacks, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess,
    CPingResponse, CPluginMessage, CRegistryData, CRespawn, CSetCompression, CStatusResponse,
    CSystemChatMessage, CTransfer, ErasedPacket, KnownPack, OpaquePacket, Packet, Property,
    SAcknowledgeFinishConfig, SConfigPluginMessage, SEncryptionResponse, SHandshake, SKeepAlive,
    SKnownPacks, SLoginAcknowledged, SLoginPluginResponse, SLoginStart, SPingRequest,
    SPluginMessage, SStatusRequest,
};
pub use registry::{DecodedPacket, PacketRegistry, build_default_registry};
pub use version::{ConnectionState, Direction, ProtocolVersion};

/// Maximum size of a single Minecraft packet (2 MiB).
///
/// Verified by the packet decoder during framing.
pub const MAX_PACKET_SIZE: usize = 2_097_152;

/// Maximum size of decompressed packet data (8 MiB).
///
/// Protection against decompression bombs (zip bombs).
pub const MAX_PACKET_DATA_SIZE: usize = 8_388_608;

/// Protocol version of the most recent supported Minecraft release.
pub const CURRENT_MC_PROTOCOL: i32 = 774; // 1.21.11

/// Human-readable name of the most recent supported Minecraft release.
pub const CURRENT_MC_VERSION: &str = "1.21.11";
