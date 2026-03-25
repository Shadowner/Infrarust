//! Registry data for limbo and the config phase.
pub(crate) mod embedded;
pub mod extractor_format;

use crate::error::CoreError;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::ProtocolVersion;

/// Source of registry data for a given protocol version.
pub(crate) trait RegistryDataProvider: Send + Sync {
    fn registry_frames(&self, version: ProtocolVersion) -> Result<Vec<PacketFrame>, CoreError>;

    fn known_packs_frame(&self, version: ProtocolVersion)
    -> Result<Option<PacketFrame>, CoreError>;

    fn supports_version(&self, version: ProtocolVersion) -> bool;
}
