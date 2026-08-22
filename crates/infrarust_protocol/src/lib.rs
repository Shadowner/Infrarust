#[macro_use]
mod macros;

pub mod chunk;
pub mod codec;
pub mod crypto;
pub mod error;
pub mod io;
pub mod legacy;
pub mod nbt;
pub mod nbt_util;
pub mod packets;
pub mod prelude;
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
    CPingResponse, CPluginMessage, CRegistryData, CRespawn, CSetCompression, CStartConfiguration,
    CStatusResponse, CSystemChatMessage, CTransfer, DimensionInfo, ErasedPacket, KnownPack,
    OpaquePacket, Packet, Property, SAcknowledgeConfiguration, SAcknowledgeFinishConfig,
    SConfigPluginMessage, SEncryptionResponse, SHandshake, SKeepAlive, SKnownPacks,
    SLoginAcknowledged, SLoginPluginResponse, SLoginStart, SPingRequest, SPluginMessage,
    SStatusRequest,
};
pub use registry::{DecodedPacket, PacketRegistry, build_default_registry};
pub use version::{ConnectionState, Direction, ProtocolVersion};

pub const MAX_PACKET_SIZE: usize = 2_097_152;

pub const MAX_PACKET_DATA_SIZE: usize = 8_388_608;

pub const CURRENT_MC_PROTOCOL: i32 = 774;

pub const CURRENT_MC_VERSION: &str = "1.21.11";
