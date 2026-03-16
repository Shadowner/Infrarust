//! `ProviderId` — unique identifier for a server config's provenance.
//!
//! Format: `provider_type@unique_id`
//! Examples: `file@survival.toml`, `docker@mc-survival-1`

use std::fmt;
use std::str::FromStr;

use crate::error::CoreError;

/// Identifies the source of a `ServerConfig`.
///
/// Each config in the system is tagged with a `ProviderId` that tells
/// which provider created it and what unique key it uses within that provider.
///
/// The canonical string form is `provider_type@unique_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderId {
    /// Type of the provider (e.g. `"file"`, `"docker"`, `"api"`).
    pub provider_type: String,
    /// Unique identifier within the provider (e.g. `"survival.toml"`, `"mc-survival-1"`).
    pub unique_id: String,
}

impl ProviderId {
    /// Creates a new provider id.
    pub fn new(provider_type: impl Into<String>, unique_id: impl Into<String>) -> Self {
        Self {
            provider_type: provider_type.into(),
            unique_id: unique_id.into(),
        }
    }

    /// Shorthand for a file-based provider id.
    pub fn file(filename: impl Into<String>) -> Self {
        Self::new("file", filename)
    }

    /// Shorthand for a Docker-based provider id.
    pub fn docker(container_name: impl Into<String>) -> Self {
        Self::new("docker", container_name)
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.provider_type, self.unique_id)
    }
}

impl FromStr for ProviderId {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (ptype, uid) = s
            .split_once('@')
            .ok_or_else(|| CoreError::InvalidProviderId(s.to_string()))?;
        Ok(Self::new(ptype, uid))
    }
}
