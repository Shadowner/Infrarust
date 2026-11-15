//! Registry data for limbo and the config phase.
pub(crate) mod embedded;
pub mod extractor_format;

use infrarust_protocol::version::ProtocolVersion;

pub fn highest_embedded_version() -> Option<ProtocolVersion> {
    embedded::highest_version()
}
