//! Loading of registry data embedded in the binary.

use std::collections::HashMap;
use std::sync::LazyLock;

use bytes::Bytes;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::ProtocolVersion;

use super::extractor_format::{ExtractedRegistryData, FrameData};
use crate::error::CoreError;

include!(concat!(env!("OUT_DIR"), "/registry_bins.rs"));

pub(crate) struct EmbeddedFrames {
    pub(crate) registry: Vec<PacketFrame>,
    pub(crate) known_packs: Option<PacketFrame>,
}

static EMBEDDED_FRAMES: LazyLock<HashMap<i32, EmbeddedFrames>> = LazyLock::new(|| {
    REGISTRY_BINS
        .iter()
        .filter_map(|bin| load_embedded(bin))
        .collect()
});

fn load_embedded(compressed: &[u8]) -> Option<(i32, EmbeddedFrames)> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut decoder = GzDecoder::new(compressed);
    let mut decompressed = Vec::new();
    if let Err(e) = decoder.read_to_end(&mut decompressed) {
        tracing::error!("Failed to decompress embedded registry data: {e}");
        return None;
    }

    match ExtractedRegistryData::from_binary(&decompressed) {
        Ok(extracted) => {
            let frame = |f: FrameData| PacketFrame::new(f.packet_id, Bytes::from(f.payload));
            Some((
                extracted.protocol_version,
                EmbeddedFrames {
                    registry: extracted.registry_frames.into_iter().map(frame).collect(),
                    known_packs: extracted.known_packs_frame.map(frame),
                },
            ))
        }
        Err(e) => {
            tracing::error!("Failed to load embedded registry data: {e}");
            None
        }
    }
}

pub(crate) fn highest_version() -> Option<ProtocolVersion> {
    EMBEDDED_FRAMES.keys().max().copied().map(ProtocolVersion)
}

pub(crate) fn frames(version: ProtocolVersion) -> Result<&'static EmbeddedFrames, CoreError> {
    EMBEDDED_FRAMES.get(&version.0).ok_or_else(|| {
        CoreError::RegistryData(format!(
            "no embedded registry data for protocol version {} ({})",
            version.0,
            version.name(),
        ))
    })
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
        let versions = ProtocolVersion::SUPPORTED
            .iter()
            .copied()
            .filter(|version| version.no_less_than(ProtocolVersion::V1_20_2));
        for version in versions {
            let embedded = frames(version).unwrap();
            let frames = &embedded.registry;
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
            for frame in frames {
                assert!(
                    required.contains(&Some(frame.id)) || Some(frame.id) == brand,
                    "{version} embeds an unexpected packet 0x{:02X}",
                    frame.id
                );
            }
            assert_eq!(
                embedded.known_packs.as_ref().map(|frame| frame.id),
                registry
                    .get_packet_id::<CKnownPacks>(version)
                    .filter(|_| version.no_less_than(ProtocolVersion::V1_20_5)),
                "{version}"
            );
        }
    }
}
