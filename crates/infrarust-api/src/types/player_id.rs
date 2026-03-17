//! Player identifier type.

use std::fmt;

/// Unique identifier for a player within the current proxy session.
///
/// This is an opaque handle assigned by the proxy — it is **not** the
/// Minecraft UUID. Use [`GameProfile::uuid`](super::GameProfile::uuid)
/// for the Mojang UUID.
///
/// # Example
/// ```
/// use infrarust_api::types::PlayerId;
///
/// let id = PlayerId::new(42);
/// assert_eq!(id.as_u64(), 42);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerId(u64);

impl PlayerId {
    /// Creates a new `PlayerId` from a raw `u64`.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the underlying `u64` value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Player({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn new_and_access() {
        let id = PlayerId::new(123);
        assert_eq!(id.as_u64(), 123);
    }

    #[test]
    fn display() {
        let id = PlayerId::new(7);
        assert_eq!(id.to_string(), "Player(7)");
    }

    #[test]
    fn equality_and_hash() {
        use std::collections::HashSet;
        let a = PlayerId::new(1);
        let b = PlayerId::new(1);
        let c = PlayerId::new(2);
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn copy_semantics() {
        let a = PlayerId::new(5);
        let b = a;
        assert_eq!(a, b); // a is still valid — Copy
    }
}
