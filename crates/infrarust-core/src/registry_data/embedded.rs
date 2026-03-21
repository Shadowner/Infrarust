//! Loading of registry data embedded in the binary.

use std::collections::HashMap;
use std::sync::LazyLock;

use bytes::Bytes;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::ProtocolVersion;

use super::RegistryDataProvider;
use super::extractor_format::ExtractedRegistryData;
use crate::error::CoreError;

include!(concat!(env!("OUT_DIR"), "/registry_bins.rs"));

/// Embedded registry data keyed by protocol version.
static EMBEDDED_DATA: LazyLock<HashMap<i32, ExtractedRegistryData>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    for bin in REGISTRY_BINS {
        load_embedded(&mut map, bin);
    }
    map
    
});

fn load_embedded(map: &mut HashMap<i32, ExtractedRegistryData>, data: &[u8]) {
    match ExtractedRegistryData::from_binary(data) {
        Ok(extracted) => {
            map.insert(extracted.protocol_version, extracted);
        }
        Err(e) => {
            tracing::error!("Failed to load embedded registry data: {e}");
        }
    }
}

pub(crate) struct EmbeddedRegistryDataProvider;

impl RegistryDataProvider for EmbeddedRegistryDataProvider {
    fn registry_frames(&self, version: ProtocolVersion) -> Result<Vec<PacketFrame>, CoreError> {
        let data = EMBEDDED_DATA.get(&version.0).ok_or_else(|| {
            CoreError::Other(format!(
                "No registry data available for protocol version {} ({}). \
                 Either connect a player to a real backend first, \
                 or add embedded registry data for this version.",
                version.0,
                version.name(),
            ))
        })?;

        Ok(data
            .registry_frames
            .iter()
            .map(|f| PacketFrame {
                id: f.packet_id,
                payload: Bytes::from(f.payload.clone()),
            })
            .collect())
    }

    fn known_packs_frame(
        &self,
        version: ProtocolVersion,
    ) -> Result<Option<PacketFrame>, CoreError> {
        let data = EMBEDDED_DATA.get(&version.0).ok_or_else(|| {
            CoreError::Other(format!(
                "No registry data available for protocol version {} ({}). \
                 Either connect a player to a real backend first, \
                 or add embedded registry data for this version.",
                version.0,
                version.name(),
            ))
        })?;

        Ok(data.known_packs_frame.as_ref().map(|f| PacketFrame {
            id: f.packet_id,
            payload: Bytes::from(f.payload.clone()),
        }))
    }

    fn supports_version(&self, version: ProtocolVersion) -> bool {
        EMBEDDED_DATA.contains_key(&version.0)
    }
}
