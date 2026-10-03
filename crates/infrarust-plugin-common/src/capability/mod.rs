use std::fmt;
use std::str::FromStr;

pub mod gates;

macro_rules! capabilities {
    ($($capability:ident => $name:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum Capability {
            $($capability,)*
        }

        impl Capability {
            pub const ALL: &'static [Capability] = &[$(Capability::$capability,)*];

            #[must_use]
            pub const fn to_kebab(self) -> &'static str {
                match self {
                    $(Capability::$capability => $name,)*
                }
            }
        }
    };
}

capabilities! {
    EventBus => "event-bus",
    PlayerRead => "player-read",
    PlayerWrite => "player-write",
    RawPacket => "raw-packet",
    ServerManage => "server-manage",
    Ban => "ban",
    Command => "command",
    Scheduler => "scheduler",
    ConfigRead => "config-read",
    ConfigWrite => "config-write",
    CodecFilter => "codec-filter",
    TransportFilter => "transport-filter",
    Limbo => "limbo",
    VirtualBackend => "virtual-backend",
    PermissionProvider => "permission-provider",
    FilesystemExtended => "filesystem-extended",
    Network => "network",
    ChatIntercept => "chat-intercept",
    BanProvider => "ban-provider",
    PluginMessaging => "plugin-messaging",
}

impl Capability {
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub fn from_kebab(s: &str) -> Option<Capability> {
        Self::ALL
            .iter()
            .copied()
            .find(|capability| capability.to_kebab() == s)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_kebab())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCapability(pub String);

impl fmt::Display for UnknownCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown capability `{}`", self.0)
    }
}

impl std::error::Error for UnknownCapability {}

impl FromStr for Capability {
    type Err = UnknownCapability;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_kebab(s).ok_or_else(|| UnknownCapability(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn all_follows_the_declaration_order() {
        for (i, capability) in Capability::ALL.iter().enumerate() {
            assert_eq!(capability.index(), i, "{capability:?} is out of order");
        }
    }

    #[test]
    fn kebab_names_round_trip_and_are_unique() {
        let mut names = HashSet::new();
        for &capability in Capability::ALL {
            let name = capability.to_kebab();
            assert!(names.insert(name), "duplicate kebab name: {name}");
            assert_eq!(Capability::from_kebab(name), Some(capability));
            assert_eq!(name.parse::<Capability>(), Ok(capability));
            assert_eq!(capability.to_string(), name);
        }
        assert_eq!(Capability::ALL.len(), 20);
        assert_eq!(names.len(), Capability::ALL.len());
    }

    #[test]
    fn unknown_names_are_refused() {
        assert_eq!(Capability::from_kebab("nope"), None);
        assert_eq!(Capability::from_kebab(""), None);
        assert_eq!(Capability::from_kebab("codec_filter"), None);
        assert_eq!(
            "Ban".parse::<Capability>(),
            Err(UnknownCapability("Ban".to_owned()))
        );
        assert_eq!(
            UnknownCapability("x".to_owned()).to_string(),
            "unknown capability `x`"
        );
    }
}
