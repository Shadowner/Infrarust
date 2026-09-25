use std::any::TypeId;

use crate::packets::{
    CBossBar, CChatMessageLegacy, CChunkBatchFinished, CChunkBatchStart, CChunkData, CClearTitles,
    CCommands, CConfigCookieRequest, CConfigDisconnect, CConfigPluginMessage, CConfigResourcePack,
    CConfigResourcePackPop, CConfigResourcePackPush, CConfigStoreCookie, CConfigTransfer,
    CCookieRequest, CDisconnect, CEncryptionRequest, CFinishConfig, CGameEvent, CJoinGame,
    CKeepAlive, CKnownPacks, CLoginCookieRequest, CLoginDisconnect, CLoginPluginRequest,
    CLoginSuccess, CPingResponse, CPluginMessage, CRegistryData, CResourcePack, CResourcePackPop,
    CResourcePackPush, CRespawn, CSetCenterChunk, CSetCompression, CSetDefaultSpawnPosition,
    CSetSubtitle, CSetTitle, CSetTitleTimes, CStartConfiguration, CStatusResponse, CStoreCookie,
    CSynchronizePlayerPosition, CSystemChatMessage, CTabCompleteResponse, CTabListHeaderFooter,
    CTitleLegacy, CTransfer, Packet, PacketMapping, SAcknowledgeConfiguration,
    SAcknowledgeFinishConfig, SChatAcknowledgement, SChatCommand, SChatCommandSigned, SChatMessage,
    SChatSessionUpdate, SClientInformation, SConfigClientInformation, SConfigCookieResponse,
    SConfigPluginMessage, SConfigResourcePackResponse, SCookieResponse, SEncryptionResponse,
    SHandshake, SKeepAlive, SKnownPacks, SLoginAcknowledged, SLoginCookieResponse,
    SLoginPluginResponse, SLoginStart, SPingRequest, SPluginMessage, SResourcePackResponse,
    SStatusRequest, STabCompleteRequest,
};
use crate::registry::PacketRegistry;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

pub struct PacketDescriptor {
    pub name: &'static str,
    pub state: ConnectionState,
    pub direction: Direction,
    pub encode_only: bool,
    pub ids: &'static [PacketMapping],
    pub register: fn(&mut PacketRegistry),
    pub packet_id: fn(&PacketRegistry, ProtocolVersion) -> Option<i32>,
    pub type_id: fn() -> TypeId,
}

macro_rules! packet_table {
    ( $( $packet:ty ),* $(,)? ) => {
        pub const DEFAULT_PACKETS: &[PacketDescriptor] = &[
            $( PacketDescriptor {
                name: <$packet as Packet>::NAME,
                state: <$packet as Packet>::STATE,
                direction: <$packet as Packet>::DIRECTION,
                encode_only: <$packet as Packet>::ENCODE_ONLY,
                ids: <$packet as Packet>::IDS,
                register: PacketRegistry::register::<$packet>,
                packet_id: PacketRegistry::get_packet_id::<$packet>,
                type_id: TypeId::of::<$packet>,
            } ),*
        ];
    };
}

packet_table! {
    SHandshake,
    SStatusRequest,
    SPingRequest,
    CStatusResponse,
    CPingResponse,
    SLoginStart,
    SEncryptionResponse,
    SLoginPluginResponse,
    SLoginAcknowledged,
    CLoginDisconnect,
    CEncryptionRequest,
    CLoginSuccess,
    CSetCompression,
    CLoginPluginRequest,
    CLoginCookieRequest,
    SLoginCookieResponse,
    SConfigPluginMessage,
    SAcknowledgeFinishConfig,
    SKnownPacks,
    SConfigClientInformation,
    CConfigPluginMessage,
    CConfigDisconnect,
    CFinishConfig,
    CRegistryData,
    CKnownPacks,
    CConfigResourcePack,
    CConfigResourcePackPush,
    CConfigResourcePackPop,
    CConfigStoreCookie,
    CConfigCookieRequest,
    CConfigTransfer,
    SConfigResourcePackResponse,
    SConfigCookieResponse,
    CCommands,
    CTabCompleteResponse,
    CKeepAlive,
    CDisconnect,
    CJoinGame,
    CRespawn,
    CPluginMessage,
    CChatMessageLegacy,
    CSystemChatMessage,
    CTitleLegacy,
    CSetTitle,
    CSetSubtitle,
    CSetTitleTimes,
    CTransfer,
    CClearTitles,
    CTabListHeaderFooter,
    CBossBar,
    CResourcePack,
    CResourcePackPush,
    CResourcePackPop,
    CStoreCookie,
    CCookieRequest,
    CStartConfiguration,
    CGameEvent,
    CSetCenterChunk,
    CChunkBatchStart,
    CChunkBatchFinished,
    CChunkData,
    CSetDefaultSpawnPosition,
    CSynchronizePlayerPosition,
    STabCompleteRequest,
    SAcknowledgeConfiguration,
    SKeepAlive,
    SChatMessage,
    SChatCommand,
    SChatCommandSigned,
    SChatAcknowledgement,
    SChatSessionUpdate,
    SClientInformation,
    SPluginMessage,
    SResourcePackResponse,
    SCookieResponse,
}

#[must_use]
pub fn build_default_registry() -> PacketRegistry {
    let mut registry = PacketRegistry::new();
    for descriptor in DEFAULT_PACKETS {
        (descriptor.register)(&mut registry);
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn default_packet_table_is_well_formed() {
        let mut seen = HashSet::new();
        for descriptor in DEFAULT_PACKETS {
            assert!(
                !descriptor.ids.is_empty(),
                "{} has an empty IDS table",
                descriptor.name
            );
            assert!(
                descriptor.ids.windows(2).all(|w| w[0].from < w[1].from),
                "{} IDS must be strictly ascending by `from`",
                descriptor.name
            );
            assert!(
                descriptor
                    .ids
                    .iter()
                    .all(|m| ProtocolVersion::SUPPORTED.contains(&m.from)),
                "{} has a `from` outside SUPPORTED",
                descriptor.name
            );
            assert!(
                descriptor
                    .ids
                    .iter()
                    .all(|m| m.to.is_none_or(|t| t >= m.from)),
                "{} has an inverted explicit range",
                descriptor.name
            );
            assert!(
                (descriptor.ids)
                    .windows(2)
                    .all(|w| w[0].id != w[1].id || w[0].to.is_some()),
                "{} has consecutive mappings with the same id",
                descriptor.name
            );
            assert!(
                seen.insert((descriptor.type_id)()),
                "{} is listed twice",
                descriptor.name
            );
        }
        assert_eq!(DEFAULT_PACKETS.len(), 75);
    }
}
