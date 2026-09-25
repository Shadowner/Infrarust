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
    ArgumentSignature, BossBarAction, CBossBar, CChatMessageLegacy, CChunkBatchFinished,
    CChunkBatchStart, CChunkData, CClearTitles, CCommands, CConfigCookieRequest, CConfigDisconnect,
    CConfigPluginMessage, CConfigResourcePack, CConfigResourcePackPop, CConfigResourcePackPush,
    CConfigStoreCookie, CConfigTransfer, CCookieRequest, CDisconnect, CEncryptionRequest,
    CFeatureFlags, CFinishConfig, CGameEvent, CJoinGame, CKeepAlive, CKnownPacks,
    CLoginCookieRequest, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CPingResponse,
    CPluginMessage, CRegistryData, CResourcePack, CResourcePackPop, CResourcePackPush, CRespawn,
    CSetCenterChunk, CSetCompression, CSetDefaultSpawnPosition, CSetSubtitle, CSetTitle,
    CSetTitleTimes, CStartConfiguration, CStatusResponse, CStoreCookie, CSynchronizePlayerPosition,
    CSystemChatMessage, CTabCompleteResponse, CTabListHeaderFooter, CTitleLegacy, CTransfer,
    CUpdateTags, ClientInformation, DimensionInfo, EncryptionProof, ErasedPacket, KnownPack,
    LastSeenMessages, MAX_COOKIE_PAYLOAD, Packet, PacketMapping, PreviousMessage, PreviousMessages,
    ProfileKey, Property, ResourcePackResult, SAcknowledgeConfiguration, SAcknowledgeFinishConfig,
    SChatAcknowledgement, SChatCommand, SChatCommandSigned, SChatMessage, SChatSessionUpdate,
    SClientInformation, SConfigClientInformation, SConfigCookieResponse, SConfigPluginMessage,
    SConfigResourcePackResponse, SCookieResponse, SEncryptionResponse, SHandshake, SKeepAlive,
    SKnownPacks, SLoginAcknowledged, SLoginCookieResponse, SLoginPluginResponse, SLoginStart,
    SPingRequest, SPluginMessage, SResourcePackResponse, SStatusRequest, STabCompleteRequest,
};
pub use registry::{DecodedPacket, PacketRegistry, build_default_registry};
pub use version::{ConnectionState, Direction, ProtocolVersion};

pub const MAX_PACKET_SIZE: usize = 2_097_152;

pub const MAX_PACKET_DATA_SIZE: usize = 8_388_608;

pub const CURRENT_MC_PROTOCOL: i32 = 774;

pub const CURRENT_MC_VERSION: &str = "1.21.11";
