//! Core provider abstractions: trait, events, and configuration wrapper.

use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_config::ServerConfig;

use crate::error::CoreError;
use crate::provider::ProviderId;

/// A server configuration tagged with its provider origin.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Unique identifier for this config (e.g. `file@survival.toml`).
    pub id: ProviderId,
    /// The server configuration.
    pub config: ServerConfig,
}

/// Event emitted by a provider when a server configuration changes.
#[derive(Debug)]
pub enum ProviderEvent {
    /// A new server configuration was discovered.
    Added(ProviderConfig),
    /// An existing server configuration was modified.
    Updated(ProviderConfig),
    /// A server configuration was removed.
    Removed(ProviderId),
}

/// Source of dynamic server configurations.
///
/// Implemented by `FileProvider`, `DockerProvider`, and future providers.
/// Each provider produces `ServerConfig` values tagged with a `ProviderId`
/// and notifies changes via a bounded channel.
///
/// Methods return boxed futures for dyn-compatibility (no `async-trait` crate).
pub trait ConfigProvider: Send + Sync {
    /// The provider type name (e.g. `"file"`, `"docker"`).
    fn provider_type(&self) -> &str;

    /// Loads the initial set of server configurations.
    ///
    /// Called once at startup. Returns all currently known configs.
    /// Individual failures (e.g. an unparseable file) should be logged
    /// and skipped, not propagated as errors.
    fn load_initial(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ProviderConfig>, CoreError>> + Send + '_>>;

    /// Watches for configuration changes and sends events.
    ///
    /// Runs until `shutdown` is cancelled. Events are sent through
    /// the bounded `sender` channel. Implementations must respect
    /// the cancellation token and exit cleanly.
    fn watch(
        &self,
        sender: mpsc::Sender<ProviderEvent>,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>>;
}
