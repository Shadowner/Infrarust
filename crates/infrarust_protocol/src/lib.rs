#[macro_use]
mod macros;

pub mod codec;
pub mod crypto;
pub mod error;
pub mod io;
pub mod legacy;
pub mod nbt;
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
    CChatMessageLegacy, CChunkBatchFinished, CChunkBatchStart, CChunkData, CCommands,
    CConfigDisconnect, CConfigPluginMessage, CDisconnect, CEncryptionRequest, CFinishConfig,
    CGameEvent, CJoinGame, CKeepAlive, CKnownPacks, CLoginDisconnect, CLoginPluginRequest,
    CLoginSuccess, CPingResponse, CPluginMessage, CRegistryData, CRespawn, CSetCenterChunk,
    CSetCompression, CSetDefaultSpawnPosition, CSetSubtitle, CSetTitle, CSetTitleTimes,
    CStartConfiguration, CStatusResponse, CSynchronizePlayerPosition, CSystemChatMessage,
    CTabCompleteResponse, CTitleLegacy, CTransfer, DimensionInfo, ErasedPacket, KnownPack, Packet,
    PacketMapping, Property, SAcknowledgeConfiguration, SAcknowledgeFinishConfig, SChatCommand,
    SChatMessage, SChatSessionUpdate, SConfigPluginMessage, SEncryptionResponse, SHandshake,
    SKeepAlive, SKnownPacks, SLoginAcknowledged, SLoginPluginResponse, SLoginStart, SPingRequest,
    SPluginMessage, SStatusRequest, STabCompleteRequest,
};
pub use registry::{DecodedPacket, PacketRegistry, build_default_registry};
pub use version::{ConnectionState, Direction, ProtocolVersion};

pub const MAX_PACKET_SIZE: usize = 2_097_152;

pub const MAX_PACKET_DATA_SIZE: usize = 8_388_608;

pub const CURRENT_MC_PROTOCOL: i32 = 774;

pub const CURRENT_MC_VERSION: &str = "1.21.11";
