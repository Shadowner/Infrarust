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

    fn as_any_mut(&mut self) -> &mut dyn Any;
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

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
