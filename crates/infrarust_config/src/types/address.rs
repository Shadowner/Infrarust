//! Server address and domain rewrite types.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::DEFAULT_MC_PORT;
use crate::error::ConfigError;

/// Address of a backend server.
///
/// Deserializes from a string `"host:port"` or `"host"` (default port = 25565).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl FromStr for ServerAddress {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Try to parse as SocketAddr first (IP:port)
        if let Ok(sock) = s.parse::<SocketAddr>() {
            return Ok(Self {
                host: sock.ip().to_string(),
                port: sock.port(),
            });
        }

        // Otherwise, split on the last ':'
        if let Some((host, port_str)) = s.rsplit_once(':')
            && let Ok(port) = port_str.parse::<u16>()
        {
            return Ok(Self {
                host: host.to_string(),
                port,
            });
        }

        // No port → default 25565
        if s.is_empty() {
            return Err(ConfigError::InvalidAddress(s.to_string()));
        }

        Ok(Self {
            host: s.to_string(),
            port: DEFAULT_MC_PORT,
        })
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl Serialize for ServerAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Serde deserialization from a string.
impl<'de> Deserialize<'de> for ServerAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// How to rewrite the domain in the Minecraft handshake
/// before forwarding it to the backend.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DomainRewrite {
    /// No rewrite — the original domain is forwarded as-is.
    #[default]
    None,
    /// Rewrites with an explicit domain.
    Explicit(String),
    /// Extracts the domain from the first backend address.
    FromBackend,
}
