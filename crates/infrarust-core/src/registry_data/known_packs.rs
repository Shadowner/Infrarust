//! KnownPacks-based registry data provider for >= 1.20.5.

use std::sync::Arc;

use bytes::Bytes;
use infrarust_protocol::codec::{McBufWriteExt, VarInt};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{CKnownPacks, CRegistryData, KnownPack};
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use super::entry_lists;
use super::RegistryDataProvider;
use crate::error::CoreError;

pub(crate) struct KnownPacksProvider {
    packet_registry: Arc<PacketRegistry>,
}

impl KnownPacksProvider {
    pub fn new(packet_registry: Arc<PacketRegistry>) -> Self {
        Self { packet_registry }
    }

    fn registry_data_packet_id(&self, version: ProtocolVersion) -> Result<i32, CoreError> {
        self.packet_registry
            .get_packet_id::<CRegistryData>(
                ConnectionState::Config,
                Direction::Clientbound,
                version,
            )
            .ok_or_else(|| {
                CoreError::Other(format!(
                    "no packet ID for CRegistryData in Config/Clientbound/{version:?}"
                ))
            })
    }
}

impl RegistryDataProvider for KnownPacksProvider {
    fn registry_frames(&self, version: ProtocolVersion) -> Result<Vec<PacketFrame>, CoreError> {
        let entries = entry_lists::get_entries(version).ok_or_else(|| {
            CoreError::Other(format!(
                "no KnownPacks entry list for protocol version {} ({})",
                version.0,
                version.name(),
            ))
        })?;

        let packet_id = self.registry_data_packet_id(version)?;
        let registries = entries.registries(version);
        let mut frames = Vec::with_capacity(registries.len());

        for (registry_id, entry_names) in registries {
            let frame = build_no_nbt_frame(registry_id, entry_names, packet_id)?;
            frames.push(frame);
        }

        Ok(frames)
    }

    fn known_packs_frame(&self, version: ProtocolVersion) -> Result<Option<PacketFrame>, CoreError> {
        if version.less_than(ProtocolVersion::V1_20_5) {
            return Ok(None);
        }

        let pkt = CKnownPacks {
            packs: vec![KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: version.name().into(),
            }],
        };

        let mut payload = Vec::new();
        pkt.encode(&mut payload, version)
            .map_err(|e| CoreError::Other(e.to_string()))?;

        let packet_id = self
            .packet_registry
            .get_packet_id::<CKnownPacks>(
                ConnectionState::Config,
                Direction::Clientbound,
                version,
            )
            .ok_or_else(|| {
                CoreError::Other(format!(
                    "no packet ID for CKnownPacks in Config/Clientbound/{version:?}"
                ))
            })?;

        Ok(Some(PacketFrame {
            id: packet_id,
            payload: Bytes::from(payload),
        }))
    }

    fn supports_version(&self, version: ProtocolVersion) -> bool {
        version.no_less_than(ProtocolVersion::V1_20_5)
            && entry_lists::get_entries(version).is_some()
    }
}

fn build_no_nbt_frame(
    registry_id: &str,
    entry_names: &[&str],
    packet_id: i32,
) -> Result<PacketFrame, CoreError> {
    let mut buf = Vec::new();

    buf.write_string(registry_id)
        .map_err(|e| CoreError::Other(e.to_string()))?;

    buf.write_var_int(&VarInt(entry_names.len() as i32))
        .map_err(|e| CoreError::Other(e.to_string()))?;

    for name in entry_names {
        buf.write_string(name)
            .map_err(|e| CoreError::Other(e.to_string()))?;
        buf.write_bool(false)
            .map_err(|e| CoreError::Other(e.to_string()))?;
    }

    Ok(PacketFrame {
        id: packet_id,
        payload: Bytes::from(buf),
    })
}
