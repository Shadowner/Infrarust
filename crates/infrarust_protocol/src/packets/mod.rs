//! Minecraft packet definitions organized by connection state (handshake, login, status, config, play).
//!
//! Each packet implements the [`Packet`] trait for version-aware encoding and decoding,
//! with packet IDs managed externally by the registry rather than embedded in the type.

/// Generates a Clientbound + Serverbound packet pair that share identical fields
/// and encode/decode logic, differing only in name and direction.
///
/// Parameter names for `decode` and `encode` are passed through the macro to satisfy
/// Rust 2024 macro hygiene (the body can only reference identifiers it receives).
///
/// # Example
/// ```ignore
/// define_twin_packets! {
///     clientbound: CMyPacket,
///     serverbound: SMyPacket,
///     state: ConnectionState::Play,
///     fields: { pub value: i64 },
///     decode(r, _version): { Ok(Self { value: r.read_i64_be()? }) },
///     encode(self, w, _version): { w.write_i64_be(self.value)?; Ok(()) },
/// }
/// ```
macro_rules! define_twin_packets {
    (
        clientbound: $c_name:ident,
        serverbound: $s_name:ident,
        state: $state:expr,
        fields: { $( pub $field:ident : $ty:ty ),* $(,)? },
        decode($r:ident, $decode_ver:ident): $decode_body:expr,
        encode($self_:ident, $w:ident, $encode_ver:ident): $encode_body:expr $(,)?
    ) => {
        #[derive(Debug, Clone)]
        pub struct $c_name {
            $( pub $field : $ty, )*
        }

        impl $crate::packets::Packet for $c_name {
            const NAME: &'static str = stringify!($c_name);

            fn state() -> $crate::version::ConnectionState { $state }
            fn direction() -> $crate::version::Direction {
                $crate::version::Direction::Clientbound
            }

            fn decode($r: &mut &[u8], $decode_ver: $crate::version::ProtocolVersion)
                -> $crate::error::ProtocolResult<Self>
            {
                $decode_body
            }

            #[allow(unused_mut)]
            fn encode(
                &$self_,
                mut $w: &mut (impl std::io::Write + ?Sized),
                $encode_ver: $crate::version::ProtocolVersion,
            ) -> $crate::error::ProtocolResult<()> {
                $encode_body
            }
        }

        #[derive(Debug, Clone)]
        pub struct $s_name {
            $( pub $field : $ty, )*
        }

        impl $crate::packets::Packet for $s_name {
            const NAME: &'static str = stringify!($s_name);

            fn state() -> $crate::version::ConnectionState { $state }
            fn direction() -> $crate::version::Direction {
                $crate::version::Direction::Serverbound
            }

            fn decode($r: &mut &[u8], $decode_ver: $crate::version::ProtocolVersion)
                -> $crate::error::ProtocolResult<Self>
            {
                $decode_body
            }

            #[allow(unused_mut)]
            fn encode(
                &$self_,
                mut $w: &mut (impl std::io::Write + ?Sized),
                $encode_ver: $crate::version::ProtocolVersion,
            ) -> $crate::error::ProtocolResult<()> {
                $encode_body
            }
        }
    };
}

pub mod config;
pub mod handshake;
pub mod login;
pub mod opaque;
pub mod play;
pub mod status;

pub use config::{
    CConfigDisconnect, CConfigPluginMessage, CFinishConfig, CKnownPacks, CRegistryData, KnownPack,
    SAcknowledgeFinishConfig, SConfigPluginMessage, SKnownPacks,
};
pub use handshake::SHandshake;
pub use login::{
    CEncryptionRequest, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CSetCompression,
    Property, SEncryptionResponse, SLoginAcknowledged, SLoginPluginResponse, SLoginStart,
};
pub use opaque::OpaquePacket;
pub use play::{
    CChatMessageLegacy, CChunkBatchFinished, CChunkBatchStart, CCommands, CDisconnect, CGameEvent,
    CJoinGame, CKeepAlive, CPluginMessage, CRespawn, CSetCenterChunk, CSetDefaultSpawnPosition,
    CSetSubtitle, CSetTitle, CSetTitleTimes, CStartConfiguration, CSynchronizePlayerPosition,
    CSystemChatMessage, CTabCompleteResponse, CTitleLegacy, CTransfer, DimensionInfo,
    SAcknowledgeConfiguration, SChatCommand, SChatMessage, SChatSessionUpdate, SKeepAlive,
    SPluginMessage, STabCompleteRequest,
};
pub use status::{CPingResponse, CStatusResponse, SPingRequest, SStatusRequest};

use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};
use std::any::Any;
use std::io::Write;

/// A Minecraft packet that the proxy can encode and decode.
///
/// Key design differences from existing implementations:
///
/// - **No `const ID`** (unlike Valence) — packet IDs live in the registry,
///   not in the type. The same packet has different IDs across protocol versions.
///
/// - **`ProtocolVersion` as parameter** to encode/decode (Velocity pattern) —
///   one struct per logical packet, versioning is in the implementation.
///
/// - **Single trait** (unlike Pumpkin's ClientPacket/ServerPacket) —
///   a proxy reads AND writes in both directions.
///
/// # Example
///
/// ```
/// use infrarust_protocol::{Packet, SHandshake, VarInt};
/// use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
///
/// let handshake = SHandshake {
///     protocol_version: VarInt(769),
///     server_address: "mc.example.com".to_string(),
///     server_port: 25565,
///     next_state: ConnectionState::Login,
/// };
///
/// // Encode
/// let mut buf = Vec::new();
/// handshake.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
///
/// // Decode
/// let decoded = SHandshake::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
/// assert_eq!(decoded.server_address, "mc.example.com");
/// assert_eq!(decoded.server_port, 25565);
/// ```
pub trait Packet: Send + Sync + std::fmt::Debug + 'static {
    /// Human-readable name for logging and debug.
    const NAME: &'static str;

    /// The connection state in which this packet is valid.
    fn state() -> ConnectionState;

    /// The direction of this packet (Serverbound or Clientbound).
    fn direction() -> Direction;

    /// Decodes the packet payload.
    ///
    /// `r` contains the bytes AFTER the `packet_id` (already read by framing).
    /// `version` is the protocol version of the current connection.
    ///
    /// # Errors
    /// Returns an error if the payload is incomplete, corrupted, or invalid.
    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self>
    where
        Self: Sized;

    /// Encodes the packet payload.
    ///
    /// Writes the bytes WITHOUT the `packet_id` (added by the encoder/registry).
    /// `version` is the protocol version of the destination connection.
    ///
    /// # Errors
    /// Returns an error if writing to `w` fails or the packet data is invalid.
    fn encode(&self, w: &mut (impl Write + ?Sized), version: ProtocolVersion)
    -> ProtocolResult<()>;
}

/// Object-safe version of the [`Packet`] trait.
///
/// Used for type-erasure in the registry: when decoding a packet,
/// we get a `Box<dyn ErasedPacket>` that can be downcast to the
/// concrete type via [`as_any()`](ErasedPacket::as_any).
pub trait ErasedPacket: Send + Sync + std::fmt::Debug {
    /// Human-readable packet name.
    fn packet_name(&self) -> &'static str;

    /// Encodes the payload into the given writer.
    ///
    /// # Errors
    /// Returns an error if writing to `w` fails or the packet data is invalid.
    fn encode_payload(&self, w: &mut dyn Write, version: ProtocolVersion) -> ProtocolResult<()>;

    /// Allows downcasting to the concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Allows mutable downcasting to the concrete type.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Blanket impl: any `Packet + Any` automatically gains `ErasedPacket`.
impl<P: Packet + Any> ErasedPacket for P {
    fn packet_name(&self) -> &'static str {
        P::NAME
    }

    fn encode_payload(&self, w: &mut dyn Write, version: ProtocolVersion) -> ProtocolResult<()> {
        self.encode(w, version)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
