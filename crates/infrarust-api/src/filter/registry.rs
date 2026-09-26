//! Sealed registry traits for filter registration.
//!
//! Only the proxy core implements these traits. Plugins use them
//! via [`PluginContext::codec_filters()`] and [`PluginContext::transport_filters()`].

use super::codec::CodecFilterFactory;
use super::transport::TransportFilter;

pub mod private {
    /// Sealed — only the proxy implements these registries.
    pub trait Sealed {}
}

pub const PROXY_FILTER_OWNER: &str = "infrarust";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FilterRegistryError {
    #[error("filter '{id}' is owned by '{owner}'")]
    OwnedBy { id: String, owner: String },
    #[error("no filter '{0}' is registered")]
    NotFound(String),
}

/// Registry for [`CodecFilterFactory`] instances.
///
/// Plugins register factories here; the proxy creates per-connection
/// instances from them when a new session is established.
pub trait CodecFilterRegistry: Send + Sync + private::Sealed {
    /// Registers a codec filter factory.
    fn register(&self, factory: Box<dyn CodecFilterFactory>) -> Result<(), FilterRegistryError>;

    /// Removes a codec filter factory by its metadata id.
    fn unregister(&self, filter_id: &str) -> Result<(), FilterRegistryError>;
}

/// Registry for [`TransportFilter`] instances.
///
/// Plugins register transport filters here; the proxy calls them
/// for every accepted TCP connection.
pub trait TransportFilterRegistry: Send + Sync + private::Sealed {
    /// Registers a transport filter.
    fn register(&self, filter: Box<dyn TransportFilter>) -> Result<(), FilterRegistryError>;

    /// Removes a transport filter by its metadata id.
    fn unregister(&self, filter_id: &str) -> Result<(), FilterRegistryError>;
}
