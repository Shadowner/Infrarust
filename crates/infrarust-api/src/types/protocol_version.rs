//! Minecraft protocol version type.

use std::fmt;

/// A Minecraft protocol version number.
///
/// Wraps the integer protocol version used during the handshake.
/// Named constants are provided for common Minecraft versions.
///
/// # Example
/// ```
/// use infrarust_api::types::ProtocolVersion;
///
/// let version = ProtocolVersion::MINECRAFT_1_21;
/// assert!(version.raw() > 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProtocolVersion(i32);

impl ProtocolVersion {
    // --- Named constants for common Minecraft versions ---

    /// Minecraft 1.20.4 / 1.20.3 (protocol 765)
    pub const MINECRAFT_1_20_4: Self = Self(765);
    /// Minecraft 1.20.5 / 1.20.6 (protocol 766)
    pub const MINECRAFT_1_20_6: Self = Self(766);
    /// Minecraft 1.21 (protocol 767)
    pub const MINECRAFT_1_21: Self = Self(767);
    /// Minecraft 1.21.2 / 1.21.3 (protocol 768)
    pub const MINECRAFT_1_21_2: Self = Self(768);
    /// Minecraft 1.21.4 (protocol 769)
    pub const MINECRAFT_1_21_4: Self = Self(769);

    /// The minimum protocol version supported by this proxy.
    pub const MINIMUM_SUPPORTED: Self = Self::MINECRAFT_1_20_4;
    /// The maximum protocol version supported by this proxy.
    pub const MAXIMUM_SUPPORTED: Self = Self::MINECRAFT_1_21_4;

    /// Creates a `ProtocolVersion` from a raw protocol number.
    pub const fn new(version: i32) -> Self {
        Self(version)
    }

    /// Returns the raw protocol version number.
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Returns `true` if this version is within the supported range.
    pub const fn is_supported(self) -> bool {
        self.0 >= Self::MINIMUM_SUPPORTED.0 && self.0 <= Self::MAXIMUM_SUPPORTED.0
    }

    /// Returns `true` if this version is at least the given version.
    pub const fn at_least(self, other: Self) -> bool {
        self.0 >= other.0
    }

    /// Returns `true` if this version is at most the given version.
    pub const fn at_most(self, other: Self) -> bool {
        self.0 <= other.0
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MINECRAFT_1_20_4 => write!(f, "1.20.4 (765)"),
            Self::MINECRAFT_1_20_6 => write!(f, "1.20.6 (766)"),
            Self::MINECRAFT_1_21 => write!(f, "1.21 (767)"),
            Self::MINECRAFT_1_21_2 => write!(f, "1.21.2 (768)"),
            Self::MINECRAFT_1_21_4 => write!(f, "1.21.4 (769)"),
            Self(v) => write!(f, "Unknown({v})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_constants() {
        assert_eq!(ProtocolVersion::MINECRAFT_1_21.raw(), 767);
        assert_eq!(ProtocolVersion::MINECRAFT_1_20_4.raw(), 765);
    }

    #[test]
    fn is_supported() {
        assert!(ProtocolVersion::MINECRAFT_1_21.is_supported());
        assert!(!ProtocolVersion::new(1).is_supported());
    }

    #[test]
    fn comparison() {
        assert!(ProtocolVersion::MINECRAFT_1_21.at_least(ProtocolVersion::MINECRAFT_1_20_4));
        assert!(ProtocolVersion::MINECRAFT_1_20_4.at_most(ProtocolVersion::MINECRAFT_1_21));
        assert!(ProtocolVersion::MINECRAFT_1_21 > ProtocolVersion::MINECRAFT_1_20_4);
    }

    #[test]
    fn display_known() {
        assert_eq!(ProtocolVersion::MINECRAFT_1_21.to_string(), "1.21 (767)");
    }

    #[test]
    fn display_unknown() {
        assert_eq!(ProtocolVersion::new(999).to_string(), "Unknown(999)");
    }

    #[test]
    fn copy_semantics() {
        let a = ProtocolVersion::MINECRAFT_1_21;
        let b = a;
        assert_eq!(a, b);
    }
}
