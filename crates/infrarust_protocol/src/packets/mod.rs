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

pub trait Packet: Send + Sync + std::fmt::Debug + 'static {
    const NAME: &'static str;

    fn state() -> ConnectionState;

    fn direction() -> Direction;

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
