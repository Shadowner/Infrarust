//! Protocol version, connection state, and packet direction types.
//!
//! These types form the foundation of the packet registry system, enabling
//! version-aware packet encoding/decoding across the full range of supported
//! Minecraft protocol versions.

use std::fmt;

/// A Minecraft protocol version identifier.
///
/// Wraps the numeric protocol ID (e.g., 767 for Minecraft 1.21).
/// The natural ordering matches protocol IDs: newer versions are greater.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub i32);

impl ProtocolVersion {
    /// Unknown protocol version.
    pub const UNKNOWN: Self = Self(-1);
    /// Legacy protocol (Beta 1.8 to MC 1.6, pre-Netty).
    pub const LEGACY: Self = Self(0);
    /// Minecraft 1.7.2
    pub const V1_7_2: Self = Self(4);
    /// Minecraft 1.7.6
    pub const V1_7_6: Self = Self(5);
    /// Minecraft 1.8
    pub const V1_8: Self = Self(47);
    /// Minecraft 1.9
    pub const V1_9: Self = Self(107);
    /// Minecraft 1.9.2
    pub const V1_9_2: Self = Self(109);
    /// Minecraft 1.9.4
    pub const V1_9_4: Self = Self(110);
    /// Minecraft 1.12
    pub const V1_12: Self = Self(335);
    /// Minecraft 1.12.1
    pub const V1_12_1: Self = Self(338);
    /// Minecraft 1.12.2
    pub const V1_12_2: Self = Self(340);
    /// Minecraft 1.13
    pub const V1_13: Self = Self(393);
    /// Minecraft 1.14
    pub const V1_14: Self = Self(477);
    /// Minecraft 1.15
    pub const V1_15: Self = Self(573);
    /// Minecraft 1.16
    pub const V1_16: Self = Self(735);
    /// Minecraft 1.16.2
    pub const V1_16_2: Self = Self(751);
    /// Minecraft 1.16.4
    pub const V1_16_4: Self = Self(754);
    /// Minecraft 1.17
    pub const V1_17: Self = Self(755);
    /// Minecraft 1.18
    pub const V1_18: Self = Self(757);
    /// Minecraft 1.18.2
    pub const V1_18_2: Self = Self(758);
    /// Minecraft 1.19
    pub const V1_19: Self = Self(759);
    /// Minecraft 1.19.1
    pub const V1_19_1: Self = Self(760);
    /// Minecraft 1.19.3
    pub const V1_19_3: Self = Self(761);
    /// Minecraft 1.19.4
    pub const V1_19_4: Self = Self(762);
    /// Minecraft 1.20
    pub const V1_20: Self = Self(763);
    /// Minecraft 1.20.2
    pub const V1_20_2: Self = Self(764);
    /// Minecraft 1.20.3
    pub const V1_20_3: Self = Self(765);
    /// Minecraft 1.20.5
    pub const V1_20_5: Self = Self(766);
    /// Minecraft 1.21
    pub const V1_21: Self = Self(767);
    /// Minecraft 1.21.2
    pub const V1_21_2: Self = Self(768);
    /// Minecraft 1.21.4
    pub const V1_21_4: Self = Self(769);
    /// Minecraft 1.21.5
    pub const V1_21_5: Self = Self(770);
    /// Minecraft 1.21.6
    pub const V1_21_6: Self = Self(771);
    /// Minecraft 1.21.7
    pub const V1_21_7: Self = Self(772);
    /// Minecraft 1.21.9
    pub const V1_21_9: Self = Self(773);
    /// Minecraft 1.21.11
    pub const V1_21_11: Self = Self(774);

    /// All supported protocol versions, sorted in ascending order.
    ///
    /// Excludes [`UNKNOWN`](Self::UNKNOWN) and [`LEGACY`](Self::LEGACY) which are
    /// special-case values. Used by the packet registry to iterate over version ranges.
    pub const SUPPORTED: &[Self] = &[
        Self::V1_7_2,
        Self::V1_7_6,
        Self::V1_8,
        Self::V1_9,
        Self::V1_9_2,
        Self::V1_9_4,
        Self::V1_12,
        Self::V1_12_1,
        Self::V1_12_2,
        Self::V1_13,
        Self::V1_14,
        Self::V1_15,
        Self::V1_16,
        Self::V1_16_2,
        Self::V1_16_4,
        Self::V1_17,
        Self::V1_18,
        Self::V1_18_2,
        Self::V1_19,
        Self::V1_19_1,
        Self::V1_19_3,
        Self::V1_19_4,
        Self::V1_20,
        Self::V1_20_2,
        Self::V1_20_3,
        Self::V1_20_5,
        Self::V1_21,
        Self::V1_21_2,
        Self::V1_21_4,
        Self::V1_21_5,
        Self::V1_21_6,
        Self::V1_21_7,
        Self::V1_21_9,
        Self::V1_21_11,
    ];

    /// Returns `true` if `self >= other`.
    pub fn no_less_than(self, other: Self) -> bool {
        self >= other
    }

    /// Returns `true` if `self <= other`.
    pub fn no_greater_than(self, other: Self) -> bool {
        self <= other
    }

    /// Returns `true` if `self < other`.
    pub fn less_than(self, other: Self) -> bool {
        self < other
    }

    /// Returns `true` if `self > other`.
    pub fn greater_than(self, other: Self) -> bool {
        self > other
    }

    /// Returns `true` if this is a legacy (pre-Netty) protocol version.
    ///
    /// Legacy versions (Beta 1.8 through MC 1.6) use a different wire format
    /// and are handled by a separate code path.
    pub const fn is_legacy(self) -> bool {
        self.0 <= Self::LEGACY.0 && self.0 >= 0
    }

    /// Returns `true` if this version is unknown.
    pub fn is_unknown(self) -> bool {
        self == Self::UNKNOWN
    }

    /// Returns the human-readable Minecraft version name.
    ///
    /// Returns `"unknown"` for unrecognized protocol IDs.
    pub const fn name(self) -> &'static str {
        match self {
            Self::LEGACY => "legacy",
            Self::V1_7_2 => "1.7.2",
            Self::V1_7_6 => "1.7.6",
            Self::V1_8 => "1.8",
            Self::V1_9 => "1.9",
            Self::V1_9_2 => "1.9.2",
            Self::V1_9_4 => "1.9.4",
            Self::V1_12 => "1.12",
            Self::V1_12_1 => "1.12.1",
            Self::V1_12_2 => "1.12.2",
            Self::V1_13 => "1.13",
            Self::V1_14 => "1.14",
            Self::V1_15 => "1.15",
            Self::V1_16 => "1.16",
            Self::V1_16_2 => "1.16.2",
            Self::V1_16_4 => "1.16.4",
            Self::V1_17 => "1.17",
            Self::V1_18 => "1.18",
            Self::V1_18_2 => "1.18.2",
            Self::V1_19 => "1.19",
            Self::V1_19_1 => "1.19.1",
            Self::V1_19_3 => "1.19.3",
            Self::V1_19_4 => "1.19.4",
            Self::V1_20 => "1.20",
            Self::V1_20_2 => "1.20.2",
            Self::V1_20_3 => "1.20.3",
            Self::V1_20_5 => "1.20.5",
            Self::V1_21 => "1.21",
            Self::V1_21_2 => "1.21.2",
            Self::V1_21_4 => "1.21.4",
            Self::V1_21_5 => "1.21.5",
            Self::V1_21_6 => "1.21.6",
            Self::V1_21_7 => "1.21.7",
            Self::V1_21_9 => "1.21.9",
            Self::V1_21_11 => "1.21.11",
            _ => "unknown",
        }
    }

    /// Iterates over supported versions in the inclusive range `[from, to]`.
    ///
    /// Returns an empty iterator if `from > to`. This is the Rust equivalent
    /// of Velocity's `EnumSet.range(from, to)`, used by the packet registry
    /// to populate version-specific mappings.
    pub fn range(from: Self, to: Self) -> impl Iterator<Item = Self> {
        Self::SUPPORTED
            .iter()
            .copied()
            .filter(move |v| v.no_less_than(from) && v.no_greater_than(to))
    }

    /// Returns the number of supported versions in the inclusive range `[from, to]`.
    ///
    /// Useful for pre-allocating collections in the packet registry.
    pub fn range_count(from: Self, to: Self) -> usize {
        Self::SUPPORTED
            .iter()
            .filter(|v| v.no_less_than(from) && v.no_greater_than(to))
            .count()
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.name();
        if name == "unknown" {
            write!(f, "protocol:{}", self.0)
        } else {
            f.write_str(name)
        }
    }
}

/// The state of a Minecraft protocol connection.
///
/// Connections progress through states: Handshake → Status or Login → (Config →) Play.
/// Each state has its own set of valid packet IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// Initial state. The client sends a handshake packet to declare intent.
    Handshake,
    /// Server list ping / status query.
    Status,
    /// Authentication and login flow.
    Login,
    /// Configuration state, added in 1.20.2 (protocol 764).
    Config,
    /// Main gameplay state.
    Play,
}

impl ConnectionState {
    /// Returns the numeric ID used in the Handshake packet's `next_state` field.
    ///
    /// - `Status` → `Some(1)`
    /// - `Login` → `Some(2)`
    /// - `Handshake` → `None` (it is the initial state, never specified as a target)
    /// - `Config` → `None` (reached from Login, not directly from Handshake)
    /// - `Play` → `None` (reached from Login/Config, not directly from Handshake)
    ///
    /// Note: Transfer (intention 3) is handled by [`from_handshake_id`](Self::from_handshake_id)
    /// but does not have its own variant yet.
    pub const fn handshake_id(self) -> Option<i32> {
        match self {
            Self::Status => Some(1),
            Self::Login => Some(2),
            Self::Handshake | Self::Config | Self::Play => None,
        }
    }

    /// Resolves a Handshake packet's `next_state` field to a [`ConnectionState`].
    ///
    /// - `1` → `Status`
    /// - `2` → `Login`
    /// - `3` → `Login` (Transfer intent, introduced in 1.20.5. Mapped to Login because
    ///   the subsequent packet flow is identical. A dedicated `Transfer` variant may be
    ///   added in the future if the flows diverge.)
    /// - anything else → `None`
    pub const fn from_handshake_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Status),
            // Transfer (intent 3) was added in 1.20.5 (protocol 766).
            // The login flow after a Transfer handshake is identical to a normal Login,
            // so we map it to Login for now. If Transfer-specific behavior is needed,
            // a dedicated variant can be introduced.
            2 | 3 => Some(Self::Login),
            _ => None,
        }
    }
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Handshake => "handshake",
            Self::Status => "status",
            Self::Login => "login",
            Self::Config => "config",
            Self::Play => "play",
        };
        f.write_str(name)
    }
}

/// The direction of a packet in the Minecraft protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Client → Server.
    Serverbound,
    /// Server → Client.
    Clientbound,
}

impl Direction {
    /// Returns the opposite direction.
    #[allow(clippy::return_self_not_must_use)] // Simple value type, not a builder
    pub const fn opposite(self) -> Self {
        match self {
            Self::Serverbound => Self::Clientbound,
            Self::Clientbound => Self::Serverbound,
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Serverbound => "serverbound",
            Self::Clientbound => "clientbound",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;


    #[test]
    fn test_version_ordering_matches_protocol_ids() {
        assert!(ProtocolVersion::V1_21 > ProtocolVersion::V1_20);
        assert!(ProtocolVersion::V1_20 > ProtocolVersion::V1_8);
        assert!(ProtocolVersion::V1_8 > ProtocolVersion::V1_7_2);
    }

    #[test]
    fn test_version_equality() {
        assert_eq!(ProtocolVersion(767), ProtocolVersion(767));
        assert_eq!(ProtocolVersion::V1_21, ProtocolVersion(767));
    }

    #[test]
    fn test_version_range_returns_inclusive_bounds() {
        let versions: Vec<_> =
            ProtocolVersion::range(ProtocolVersion::V1_19, ProtocolVersion::V1_19_4).collect();
        assert_eq!(
            versions,
            vec![
                ProtocolVersion::V1_19,
                ProtocolVersion::V1_19_1,
                ProtocolVersion::V1_19_3,
                ProtocolVersion::V1_19_4,
            ]
        );
    }

    #[test]
    fn test_version_range_single_version() {
        let versions: Vec<_> =
            ProtocolVersion::range(ProtocolVersion::V1_21, ProtocolVersion::V1_21).collect();
        assert_eq!(versions, vec![ProtocolVersion::V1_21]);
    }

    #[test]
    fn test_version_range_empty_when_inverted() {
        assert!(
            ProtocolVersion::range(ProtocolVersion::V1_20, ProtocolVersion::V1_19)
                .next()
                .is_none()
        );
    }

    #[test]
    fn test_version_range_count_matches_iterator() {
        let from = ProtocolVersion::V1_16;
        let to = ProtocolVersion::V1_19;
        assert_eq!(
            ProtocolVersion::range_count(from, to),
            ProtocolVersion::range(from, to).count()
        );
    }

    #[test]
    fn test_legacy_detection() {
        assert!(ProtocolVersion::LEGACY.is_legacy());
        assert!(!ProtocolVersion::V1_7_2.is_legacy());
        assert!(!ProtocolVersion::V1_8.is_legacy());
    }

    #[test]
    fn test_unknown_detection() {
        assert!(ProtocolVersion::UNKNOWN.is_unknown());
        assert!(!ProtocolVersion::V1_8.is_unknown());
    }

    #[test]
    fn test_comparison_methods_match_operators() {
        let pairs = [
            (ProtocolVersion::V1_8, ProtocolVersion::V1_7_2),
            (ProtocolVersion::V1_21, ProtocolVersion::V1_21),
            (ProtocolVersion::V1_19, ProtocolVersion::V1_20),
            (ProtocolVersion::V1_16, ProtocolVersion::V1_16_4),
        ];
        for (a, b) in pairs {
            assert_eq!(a.no_less_than(b), a >= b, "no_less_than({a:?}, {b:?})");
            assert_eq!(
                a.no_greater_than(b),
                a <= b,
                "no_greater_than({a:?}, {b:?})"
            );
            assert_eq!(a.less_than(b), a < b, "less_than({a:?}, {b:?})");
            assert_eq!(a.greater_than(b), a > b, "greater_than({a:?}, {b:?})");
        }
    }

    #[test]
    fn test_name_returns_human_readable() {
        assert_eq!(ProtocolVersion::V1_21.name(), "1.21");
        assert_eq!(ProtocolVersion::V1_8.name(), "1.8");
        assert_eq!(ProtocolVersion::V1_21_4.name(), "1.21.4");
        assert_eq!(ProtocolVersion::V1_12_2.name(), "1.12.2");
    }

    #[test]
    fn test_name_unknown_version_returns_unknown() {
        assert_eq!(ProtocolVersion(99999).name(), "unknown");
    }

    #[test]
    fn test_display_uses_name() {
        assert_eq!(format!("{}", ProtocolVersion::V1_21_4), "1.21.4");
        assert_eq!(format!("{}", ProtocolVersion::V1_8), "1.8");
    }

    #[test]
    fn test_display_unknown_version() {
        let display = format!("{}", ProtocolVersion(42));
        assert!(
            display.contains("protocol:42"),
            "expected 'protocol:42', got '{display}'"
        );
    }

    #[test]
    fn test_supported_is_sorted() {
        assert!(ProtocolVersion::SUPPORTED.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn test_supported_does_not_contain_unknown_or_legacy() {
        assert!(!ProtocolVersion::SUPPORTED.contains(&ProtocolVersion::UNKNOWN));
        assert!(!ProtocolVersion::SUPPORTED.contains(&ProtocolVersion::LEGACY));
    }


    #[test]
    fn test_handshake_id_status() {
        assert_eq!(ConnectionState::Status.handshake_id(), Some(1));
    }

    #[test]
    fn test_handshake_id_login() {
        assert_eq!(ConnectionState::Login.handshake_id(), Some(2));
    }

    #[test]
    fn test_handshake_id_handshake_is_none() {
        assert_eq!(ConnectionState::Handshake.handshake_id(), None);
    }

    #[test]
    fn test_from_handshake_id_round_trip() {
        for state in [ConnectionState::Status, ConnectionState::Login] {
            let id = state.handshake_id().unwrap();
            assert_eq!(ConnectionState::from_handshake_id(id), Some(state));
        }
    }

    #[test]
    fn test_from_handshake_id_invalid() {
        assert_eq!(ConnectionState::from_handshake_id(99), None);
    }

    #[test]
    fn test_from_handshake_id_transfer() {
        // Transfer (intent 3, added in 1.20.5) maps to Login for now,
        // since the subsequent packet flow is identical.
        assert_eq!(
            ConnectionState::from_handshake_id(3),
            Some(ConnectionState::Login)
        );
    }

    #[test]
    fn test_display_lowercase() {
        assert_eq!(format!("{}", ConnectionState::Play), "play");
        assert_eq!(format!("{}", ConnectionState::Handshake), "handshake");
        assert_eq!(format!("{}", ConnectionState::Status), "status");
        assert_eq!(format!("{}", ConnectionState::Login), "login");
        assert_eq!(format!("{}", ConnectionState::Config), "config");
    }


    #[test]
    fn test_opposite() {
        assert_eq!(Direction::Serverbound.opposite(), Direction::Clientbound);
        assert_eq!(Direction::Clientbound.opposite(), Direction::Serverbound);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Direction::Serverbound), "serverbound");
        assert_eq!(format!("{}", Direction::Clientbound), "clientbound");
    }
}
