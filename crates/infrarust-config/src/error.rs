//! Error types for configuration handling.

use std::path::PathBuf;

use crate::types::ProxyMode;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse TOML in {path}: {source}")]
    ParseToml {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("invalid server address: {0}")]
    InvalidAddress(String),

    #[error(
        "server '{id}' uses {proxy_mode:?} mode which requires at least one domain \
             (forwarding modes are only accessible via direct domain connection)"
    )]
    NoDomains { id: String, proxy_mode: ProxyMode },

    #[error("server config {id} has no addresses defined")]
    NoAddresses { id: String },

    #[error("duplicate config id: {0}")]
    DuplicateId(String),

    #[error("config directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    #[error("validation error: {0}")]
    Validation(String),
}
