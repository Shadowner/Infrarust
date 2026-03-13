pub mod file;

use infrarust_config::ServerConfig;

/// Describes a configuration change detected by a provider.
#[derive(Debug)]
pub enum ConfigChange {
    /// Full reload: all configs have been re-read from disk.
    FullReload(Vec<ServerConfig>),
}
