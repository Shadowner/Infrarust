//! Error types for configuration handling.

use std::path::PathBuf;

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

    #[error("server config {id} has no domains defined")]
    NoDomains { id: String },

    #[error("server config {id} has no addresses defined")]
    NoAddresses { id: String },

    #[error("duplicate config id: {0}")]
    DuplicateId(String),

    #[error("config directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    #[error("validation error: {0}")]
    Validation(String),
}
