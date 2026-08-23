use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub i32);

impl ProtocolVersion {
    pub const UNKNOWN: Self = Self(-1);
    pub const LEGACY: Self = Self(0);
    pub const V1_7_2: Self = Self(4);
    pub const V1_7_6: Self = Self(5);
    pub const V1_8: Self = Self(47);
    pub const V1_9: Self = Self(107);
    pub const V1_9_2: Self = Self(109);
    pub const V1_9_4: Self = Self(110);
    pub const V1_12: Self = Self(335);
    pub const V1_12_1: Self = Self(338);
    pub const V1_12_2: Self = Self(340);
    pub const V1_13: Self = Self(393);
    pub const V1_14: Self = Self(477);
    pub const V1_15: Self = Self(573);
    pub const V1_16: Self = Self(735);
    pub const V1_16_2: Self = Self(751);
    pub const V1_16_4: Self = Self(754);
    pub const V1_17: Self = Self(755);
    pub const V1_18: Self = Self(757);
    pub const V1_18_2: Self = Self(758);
    pub const V1_19: Self = Self(759);
    pub const V1_19_1: Self = Self(760);
    pub const V1_19_3: Self = Self(761);
    pub const V1_19_4: Self = Self(762);
    pub const V1_20: Self = Self(763);
    pub const V1_20_2: Self = Self(764);
    pub const V1_20_3: Self = Self(765);
    pub const V1_20_5: Self = Self(766);
    pub const V1_21: Self = Self(767);
    pub const V1_21_2: Self = Self(768);
    pub const V1_21_4: Self = Self(769);
    pub const V1_21_5: Self = Self(770);
    pub const V1_21_6: Self = Self(771);
    pub const V1_21_7: Self = Self(772);
    pub const V1_21_9: Self = Self(773);
    pub const V1_21_11: Self = Self(774);
    pub const V26_1: Self = Self(775);
    pub const V26_2: Self = Self(776);

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
        Self::V26_1,
        Self::V26_2,
    ];

    pub fn no_less_than(self, other: Self) -> bool {
        self >= other
    }

    pub fn no_greater_than(self, other: Self) -> bool {
        self <= other
    }

    pub fn less_than(self, other: Self) -> bool {
        self < other
    }

    pub const fn is_legacy(self) -> bool {
        self.0 <= Self::LEGACY.0 && self.0 >= 0
    }

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
            Self::V26_1 => "26.1",
            Self::V26_2 => "26.2",
            _ => "unknown",
        }
    }

    pub fn range(from: Self, to: Self) -> impl Iterator<Item = Self> {
        Self::SUPPORTED
            .iter()
            .copied()
            .filter(move |v| v.no_less_than(from) && v.no_greater_than(to))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    Handshake,
    Status,
    Login,
    Config,
    Play,
}

impl ConnectionState {
    pub const fn handshake_id(self) -> Option<i32> {
        match self {
            Self::Status => Some(1),
            Self::Login => Some(2),
            Self::Handshake | Self::Config | Self::Play => None,
        }
    }

    pub const fn from_handshake_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Status),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Serverbound,
    Clientbound,
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
    fn test_legacy_detection() {
        assert!(ProtocolVersion::LEGACY.is_legacy());
        assert!(!ProtocolVersion::V1_7_2.is_legacy());
        assert!(!ProtocolVersion::V1_8.is_legacy());
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
    fn test_display() {
        assert_eq!(format!("{}", Direction::Serverbound), "serverbound");
        assert_eq!(format!("{}", Direction::Clientbound), "clientbound");
    }
}
