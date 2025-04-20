use std::fmt;

use crate::types::VarInt;

/// Protocol version numbers for different Minecraft versions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Version(i32);

impl Version {
    /// Protocol version for Minecraft 1.18.2
    pub const V1_18_2: Version = Version(758);
    /// Protocol version for Minecraft 1.19
    pub const V1_19: Version = Version(759);
    /// Protocol version for Minecraft 1.19.3
    pub const V1_19_3: Version = Version(761);
    /// Protocol version for Minecraft 1.20.2
    pub const V1_20_2: Version = Version(764);
    /// Protocol version for Minecraft 1.21.4
    pub const V1_21_4: Version = Version(769);

    /// Creates a new Version from a protocol number
    pub const fn new(protocol: i32) -> Self {
        Version(protocol)
    }

    /// Returns the protocol number
    pub const fn protocol_number(&self) -> i32 {
        self.0
    }

    /// Returns the version name (e.g., "1.18.2")
    pub fn name(&self) -> &'static str {
        match self.0 {
            758 => "1.18.2",
            759 => "1.19",
            761 => "1.19.3",
            764 => "1.20.2",
            _ => "Unknown Version",
        }
    }

    pub fn to_varint(&self) -> VarInt {
        VarInt(self.0)
    }
}

impl From<i32> for Version {
    fn from(protocol: i32) -> Self {
        Version(protocol)
    }
}

impl From<Version> for i32 {
    fn from(version: Version) -> Self {
        version.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_constants() {
        assert_eq!(Version::V1_18_2.protocol_number(), 758);
        assert_eq!(Version::V1_19.protocol_number(), 759);
        assert_eq!(Version::V1_19_3.protocol_number(), 761);
        assert_eq!(Version::V1_20_2.protocol_number(), 764);
    }

    #[test]
    fn test_version_names() {
        assert_eq!(Version::V1_18_2.name(), "1.18.2");
        assert_eq!(Version::V1_19.name(), "1.19");
        assert_eq!(Version::V1_19_3.name(), "1.19.3");
        assert_eq!(Version::V1_20_2.name(), "1.20.2");
        assert_eq!(Version::new(0).name(), "Unknown Version");
    }

    #[test]
    fn test_version_display() {
        assert_eq!(format!("{}", Version::V1_18_2), "1.18.2");
        assert_eq!(format!("{}", Version::V1_19), "1.19");
        assert_eq!(format!("{}", Version::new(0)), "Unknown Version");
    }

    #[test]
    fn test_version_conversion() {
        let protocol_number = 758;
        let version = Version::from(protocol_number);
        assert_eq!(version, Version::V1_18_2);
        assert_eq!(i32::from(version), protocol_number);
    }

    #[test]
    fn test_version_comparison() {
        assert!(Version::V1_18_2 < Version::V1_19);
        assert!(Version::V1_19 < Version::V1_19_3);
        assert!(Version::V1_19_3 < Version::V1_20_2);
        assert_eq!(Version::V1_18_2, Version::V1_18_2);
    }
}
