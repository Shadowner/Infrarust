//! Permission type.

use std::fmt;

/// A named permission node.
///
/// Permissions are string-based identifiers used to gate access to
/// plugin functionality (e.g. `"infrarust.command.ban"`).
///
/// # Example
/// ```
/// use infrarust_api::types::Permission;
///
/// let perm = Permission::new("infrarust.admin");
/// assert_eq!(perm.as_str(), "infrarust.admin");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Permission(String);

impl Permission {
    /// Creates a new permission from a string.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the permission string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Permission {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for Permission {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn construction_and_access() {
        let p = Permission::new("test.perm");
        assert_eq!(p.as_str(), "test.perm");
        assert_eq!(p.to_string(), "test.perm");
    }

    #[test]
    fn from_str() {
        let p: Permission = "admin.kick".into();
        assert_eq!(p.as_str(), "admin.kick");
    }
}
