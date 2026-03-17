//! Server identifier type.

use std::fmt;

/// Identifier for a backend server in the proxy configuration.
///
/// This corresponds to the server name in the proxy config file
/// (e.g. `"lobby"`, `"survival"`, `"minigames"`).
///
/// # Example
/// ```
/// use infrarust_api::types::ServerId;
///
/// let id = ServerId::new("lobby");
/// assert_eq!(id.as_str(), "lobby");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerId(String);

impl ServerId {
    /// Creates a new `ServerId` from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ServerId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for ServerId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Information about a backend server.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// The server's unique identifier.
    pub id: ServerId,
    /// Optional human-readable display name.
    pub display_name: Option<String>,
    /// Network addresses for this server.
    pub addresses: Vec<ServerAddress>,
}

/// A network address for a backend server.
#[derive(Debug, Clone)]
pub struct ServerAddress {
    /// Hostname or IP address.
    pub host: String,
    /// Port number.
    pub port: u16,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn new_and_access() {
        let id = ServerId::new("lobby");
        assert_eq!(id.as_str(), "lobby");
    }

    #[test]
    fn display() {
        let id = ServerId::new("survival");
        assert_eq!(id.to_string(), "survival");
    }

    #[test]
    fn from_str() {
        let id: ServerId = "hub".into();
        assert_eq!(id.as_str(), "hub");
    }

    #[test]
    fn from_string() {
        let id: ServerId = String::from("creative").into();
        assert_eq!(id.as_str(), "creative");
    }

    #[test]
    fn equality_and_hash() {
        use std::collections::HashSet;
        let a = ServerId::new("a");
        let b = ServerId::new("a");
        let c = ServerId::new("b");
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    #[test]
    fn server_info_construction() {
        let info = ServerInfo {
            id: ServerId::new("lobby"),
            display_name: Some("Lobby".into()),
            addresses: vec![ServerAddress {
                host: "127.0.0.1".into(),
                port: 25565,
            }],
        };
        assert_eq!(info.id.as_str(), "lobby");
        assert_eq!(info.addresses.len(), 1);
        assert_eq!(info.addresses[0].port, 25565);
    }
}
