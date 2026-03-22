//! Configuration provider system.
//!
//! Providers are sources of `ServerConfig` data (files, Docker, future API).
//! Each provider implements [`ConfigProvider`] and is registered with the
//! [`ProviderRegistry`] which orchestrates loading and hot-reload.

#[cfg(feature = "docker")]
pub mod docker;
pub mod file;
pub mod plugin_adapter;
pub mod provider_id;
pub mod registry;
pub mod traits;

pub use provider_id::ProviderId;
pub use traits::*;
