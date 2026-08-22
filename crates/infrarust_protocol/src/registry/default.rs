use std::any::TypeId;

use crate::packets::{
    CChatMessageLegacy, CChunkBatchFinished, CChunkBatchStart, CChunkData, CCommands,
    CConfigDisconnect, CConfigPluginMessage, CDisconnect, CEncryptionRequest, CFinishConfig,
    CGameEvent, CJoinGame, CKeepAlive, CKnownPacks, CLoginDisconnect, CLoginPluginRequest,
    CLoginSuccess, CPingResponse, CPluginMessage, CRegistryData, CRespawn, CSetCenterChunk,
    CSetCompression, CSetDefaultSpawnPosition, CSetSubtitle, CSetTitle, CSetTitleTimes,
    CStartConfiguration, CStatusResponse, CSynchronizePlayerPosition, CSystemChatMessage,
    CTabCompleteResponse, CTitleLegacy, CTransfer, Packet, PacketMapping,
    SAcknowledgeConfiguration, SAcknowledgeFinishConfig, SChatCommand, SChatMessage,
    SChatSessionUpdate, SConfigPluginMessage, SEncryptionResponse, SHandshake, SKeepAlive,
    SKnownPacks, SLoginAcknowledged, SLoginPluginResponse, SLoginStart, SPingRequest,
    SPluginMessage, SStatusRequest, STabCompleteRequest,
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
    SConfigPluginMessage,
    SAcknowledgeFinishConfig,
    SKnownPacks,
    CConfigPluginMessage,
    CConfigDisconnect,
    CFinishConfig,
    CRegistryData,
    CKnownPacks,
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
    SChatSessionUpdate,
    SPluginMessage,
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
        assert_eq!(DEFAULT_PACKETS.len(), 51);
    }
}
