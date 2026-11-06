//! Multi-version registry data for limbo login.

use std::sync::Arc;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::ProtocolVersion;

use crate::error::CoreError;
use crate::registry_data::RegistryDataProvider;

pub struct RegistryCodecCache {
    provider: Arc<dyn RegistryDataProvider>,
}

impl RegistryCodecCache {
    pub(crate) fn new(provider: Arc<dyn RegistryDataProvider>) -> Self {
        Self { provider }
    }

    pub fn get_registry_frames(
        &self,
        version: ProtocolVersion,
    ) -> Result<Vec<PacketFrame>, CoreError> {
        self.provider.registry_frames(version)
    }

    pub fn get_known_packs_frame(
        &self,
        version: ProtocolVersion,
    ) -> Result<Option<PacketFrame>, CoreError> {
        self.provider.known_packs_frame(version)
    }

    pub fn supports_version(&self, version: ProtocolVersion) -> bool {
        self.provider.supports_version(version)
    }
}
