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

fn load_embedded(map: &mut HashMap<i32, ExtractedRegistryData>, compressed: &[u8]) {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut decoder = GzDecoder::new(compressed);
    let mut decompressed = Vec::new();
    if let Err(e) = decoder.read_to_end(&mut decompressed) {
        tracing::error!("Failed to decompress embedded registry data: {e}");
        return;
    }

    match ExtractedRegistryData::from_binary(&decompressed) {
        Ok(extracted) => {
            map.insert(extracted.protocol_version, extracted);
        }
        Err(e) => {
            tracing::error!("Failed to load embedded registry data: {e}");
        }
    }
}

pub(crate) fn highest_version() -> Option<ProtocolVersion> {
    EMBEDDED_DATA.keys().max().copied().map(ProtocolVersion)
}

pub(crate) struct EmbeddedRegistryDataProvider;

impl RegistryDataProvider for EmbeddedRegistryDataProvider {
    fn registry_frames(&self, version: ProtocolVersion) -> Result<Vec<PacketFrame>, CoreError> {
        let data = EMBEDDED_DATA.get(&version.0).ok_or_else(|| {
            CoreError::RegistryData(format!(
                "no embedded registry data for protocol version {} ({})",
                version.0,
                version.name(),
            ))
        })?;

        Ok(data
            .registry_frames
            .iter()
            .map(|f| PacketFrame::new(f.packet_id, Bytes::from(f.payload.clone())))
            .collect())
    }

    fn known_packs_frame(
        &self,
        version: ProtocolVersion,
    ) -> Result<Option<PacketFrame>, CoreError> {
        let data = EMBEDDED_DATA.get(&version.0).ok_or_else(|| {
            CoreError::RegistryData(format!(
                "no embedded registry data for protocol version {} ({})",
                version.0,
                version.name(),
            ))
        })?;

        Ok(data
            .known_packs_frame
            .as_ref()
            .map(|f| PacketFrame::new(f.packet_id, Bytes::from(f.payload.clone()))))
    }

    fn supports_version(&self, version: ProtocolVersion) -> bool {
        EMBEDDED_DATA.contains_key(&version.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_protocol::packets::config::{
        CConfigPluginMessage, CFeatureFlags, CKnownPacks, CRegistryData, CUpdateTags,
    };

    use super::*;
    use crate::limbo::test_helpers::test_registry;

    #[test]
    fn every_configuration_phase_version_has_embedded_registry_data() {
        let registry = test_registry();
        let provider = EmbeddedRegistryDataProvider;
        let versions = ProtocolVersion::SUPPORTED
            .iter()
            .copied()
            .filter(|version| version.no_less_than(ProtocolVersion::V1_20_2));
        for version in versions {
            assert!(provider.supports_version(version), "{version}");
            let frames = provider.registry_frames(version).unwrap();
            let required = [
                registry.get_packet_id::<CRegistryData>(version),
                registry.get_packet_id::<CFeatureFlags>(version),
                registry.get_packet_id::<CUpdateTags>(version),
            ];
            for id in required {
                assert!(
                    frames.iter().any(|frame| Some(frame.id) == id),
                    "{version} is missing packet {id:?}"
                );
            }
            let brand = registry.get_packet_id::<CConfigPluginMessage>(version);
            for frame in &frames {
                assert!(
                    required.contains(&Some(frame.id)) || Some(frame.id) == brand,
                    "{version} embeds an unexpected packet 0x{:02X}",
                    frame.id
                );
            }
            let known_packs = provider.known_packs_frame(version).unwrap();
            assert_eq!(
                known_packs.map(|frame| frame.id),
                registry
                    .get_packet_id::<CKnownPacks>(version)
                    .filter(|_| version.no_less_than(ProtocolVersion::V1_20_5)),
                "{version}"
            );
        }
    }
}
