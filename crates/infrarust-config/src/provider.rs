//! Trait abstraction for configuration sources.

use tokio::sync::mpsc;

use crate::error::ConfigError;
use crate::server::ServerConfig;

/// Server configuration source.
///
/// Implemented by `FileProvider`, `DockerProvider`, etc.
/// Concrete implementations live outside this crate
/// (they pull heavy dependencies like `notify` or `bollard`).
pub trait ConfigProvider: Send + Sync {
    /// Loads all server configurations.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the configuration source cannot be read
    /// or the configuration data is invalid.
    fn load_configs(&self) -> Result<Vec<ServerConfig>, ConfigError>;

    /// Subscribes to configuration changes.
    ///
    /// Returns `Some(receiver)` if the provider supports hot-reload,
    /// `None` otherwise. The receiver emits `ConfigChange` events over time.
    fn watch(&self) -> Option<mpsc::Receiver<ConfigChange>>;
}

/// A configuration change detected by a provider.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ConfigChange {
    /// A new server has been added.
    Added(ServerConfig),
    /// An existing server has been modified.
    Updated { id: String, config: ServerConfig },
    /// A server has been removed.
    Removed { id: String },
    /// Full reload (all servers).
    /// Used when the provider cannot compute a diff.
    FullReload(Vec<ServerConfig>),
}
