//! Dispatches to the right registry data provider based on protocol version.
//!
//! - `< 1.20.2`: no config phase (registry data is in JoinGame)
//! - `1.20.2 – 1.20.4`: extracted data required (`EmbeddedRegistryDataProvider`)
//! - `>= 1.20.5`: KnownPacks trick (`KnownPacksProvider`)

use std::sync::Arc;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use super::embedded::EmbeddedRegistryDataProvider;
use super::known_packs::KnownPacksProvider;
use super::RegistryDataProvider;
use crate::error::CoreError;

pub(crate) struct VersionRouter {
    embedded: EmbeddedRegistryDataProvider,
    known_packs: KnownPacksProvider,
}

impl VersionRouter {
    pub fn new(packet_registry: Arc<PacketRegistry>) -> Self {
        Self {
            embedded: EmbeddedRegistryDataProvider,
            known_packs: KnownPacksProvider::new(packet_registry),
        }
    }
}

impl RegistryDataProvider for VersionRouter {
    fn registry_frames(&self, version: ProtocolVersion) -> Result<Vec<PacketFrame>, CoreError> {
        if version.less_than(ProtocolVersion::V1_20_2) {
            return Err(CoreError::Other(
                "versions < 1.20.2 don't use config-phase registry data; \
                 registry data is embedded in JoinGame for these versions"
                    .into(),
            ));
        }

        if version.less_than(ProtocolVersion::V1_20_5) {
            self.embedded.registry_frames(version)
        } else {
            self.known_packs.registry_frames(version)
        }
    }

    fn known_packs_frame(&self, version: ProtocolVersion) -> Result<Option<PacketFrame>, CoreError> {
        if version.less_than(ProtocolVersion::V1_20_5) {
            Ok(None)
        } else {
            self.known_packs.known_packs_frame(version)
        }
    }

    fn supports_version(&self, version: ProtocolVersion) -> bool {
        if version.less_than(ProtocolVersion::V1_20_2) {
            false
        } else if version.less_than(ProtocolVersion::V1_20_5) {
            self.embedded.supports_version(version)
        } else {
            self.known_packs.supports_version(version)
        }
    }
}
