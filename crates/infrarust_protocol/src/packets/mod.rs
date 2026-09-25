pub mod config;
pub mod cookie;
pub mod handshake;
pub mod login;
pub mod play;
pub mod resource_pack;
pub mod status;

pub use config::{
    CConfigDisconnect, CConfigPluginMessage, CFinishConfig, CKnownPacks, CRegistryData, KnownPack,
    SAcknowledgeFinishConfig, SConfigClientInformation, SConfigPluginMessage, SKnownPacks,
};
pub use cookie::{
    CConfigCookieRequest, CConfigStoreCookie, CCookieRequest, CLoginCookieRequest, CStoreCookie,
    MAX_COOKIE_PAYLOAD, SConfigCookieResponse, SCookieResponse, SLoginCookieResponse,
};
pub use handshake::SHandshake;
pub use login::{
    CEncryptionRequest, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CSetCompression,
    EncryptionProof, ProfileKey, Property, SEncryptionResponse, SLoginAcknowledged,
    SLoginPluginResponse, SLoginStart,
};
pub use play::{
    ArgumentSignature, BossBarAction, CBossBar, CChatMessageLegacy, CChunkBatchFinished,
    CChunkBatchStart, CChunkData, CClearTitles, CCommands, CConfigTransfer, CDisconnect,
    CGameEvent, CJoinGame, CKeepAlive, CPluginMessage, CRespawn, CSetCenterChunk,
    CSetDefaultSpawnPosition, CSetSubtitle, CSetTitle, CSetTitleTimes, CStartConfiguration,
    CSynchronizePlayerPosition, CSystemChatMessage, CTabCompleteResponse, CTabListHeaderFooter,
    CTitleLegacy, CTransfer, ClientInformation, DimensionInfo, LastSeenMessages, PreviousMessage,
    PreviousMessages, SAcknowledgeConfiguration, SChatAcknowledgement, SChatCommand,
    SChatCommandSigned, SChatMessage, SChatSessionUpdate, SClientInformation, SKeepAlive,
    SPluginMessage, STabCompleteRequest,
};
pub use resource_pack::{
    CConfigResourcePack, CConfigResourcePackPop, CConfigResourcePackPush, CResourcePack,
    CResourcePackPop, CResourcePackPush, ResourcePackResult, SConfigResourcePackResponse,
    SResourcePackResponse,
};
pub use status::{CPingResponse, CStatusResponse, SPingRequest, SStatusRequest};

use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};
use std::any::Any;
use std::io::Write;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
    let mut buf = Vec::new();
    packet.encode(&mut buf, version).unwrap();
    P::decode(&mut buf.as_slice(), version).unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketMapping {
    pub id: i32,
    pub from: ProtocolVersion,
    pub to: Option<ProtocolVersion>,
}

pub trait Packet: Send + Sync + std::fmt::Debug + 'static {
    const NAME: &'static str;

    const STATE: ConnectionState;

    const DIRECTION: Direction;

    const IDS: &'static [PacketMapping];

    const ENCODE_ONLY: bool = false;

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self>
    where
        Self: Sized;

    fn encode(&self, w: &mut (impl Write + ?Sized), version: ProtocolVersion)
    -> ProtocolResult<()>;
}

pub trait ErasedPacket: Send + Sync + std::fmt::Debug {
    fn packet_name(&self) -> &'static str;

    fn encode_payload(&self, w: &mut dyn Write, version: ProtocolVersion) -> ProtocolResult<()>;

    fn as_any(&self) -> &dyn Any;
}

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
}
