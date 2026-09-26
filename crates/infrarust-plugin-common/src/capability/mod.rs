use std::fmt;
use std::str::FromStr;

pub mod gates;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    EventBus,
    PlayerRead,
    PlayerWrite,
    RawPacket,
    ServerManage,
    Ban,
    Command,
    Scheduler,
    ConfigRead,
    ConfigWrite,
    CodecFilter,
    TransportFilter,
    Limbo,
    VirtualBackend,
    PermissionProvider,
    FilesystemExtended,
    Network,
    ChatIntercept,
    BanProvider,
    PluginMessaging,
}

pub const TABLE: [(Capability, &str); 20] = [
    (Capability::EventBus, "event-bus"),
    (Capability::PlayerRead, "player-read"),
    (Capability::PlayerWrite, "player-write"),
    (Capability::RawPacket, "raw-packet"),
    (Capability::ServerManage, "server-manage"),
    (Capability::Ban, "ban"),
    (Capability::Command, "command"),
    (Capability::Scheduler, "scheduler"),
    (Capability::ConfigRead, "config-read"),
    (Capability::ConfigWrite, "config-write"),
    (Capability::CodecFilter, "codec-filter"),
    (Capability::TransportFilter, "transport-filter"),
    (Capability::Limbo, "limbo"),
    (Capability::VirtualBackend, "virtual-backend"),
    (Capability::PermissionProvider, "permission-provider"),
    (Capability::FilesystemExtended, "filesystem-extended"),
    (Capability::Network, "network"),
    (Capability::ChatIntercept, "chat-intercept"),
    (Capability::BanProvider, "ban-provider"),
    (Capability::PluginMessaging, "plugin-messaging"),
];

const fn all() -> [Capability; TABLE.len()] {
    let mut all = [Capability::EventBus; TABLE.len()];
    let mut i = 0;
    while i < TABLE.len() {
        all[i] = TABLE[i].0;
        i += 1;
    }
    all
}

impl Capability {
    pub const ALL: [Capability; TABLE.len()] = all();

    #[must_use]
    pub const fn to_kebab(self) -> &'static str {
        TABLE[self as usize].1
    }

    #[must_use]
    pub fn from_kebab(s: &str) -> Option<Capability> {
        TABLE
            .iter()
            .find(|(_, name)| *name == s)
            .map(|(capability, _)| *capability)
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
    fn the_table_follows_the_declaration_order() {
        for (i, (capability, _)) in TABLE.iter().enumerate() {
            assert_eq!(*capability as usize, i, "{capability:?} is out of order");
            assert_eq!(Capability::ALL[i], *capability);
        }
    }

    #[test]
    fn kebab_names_round_trip_and_are_unique() {
        let mut names = HashSet::new();
        for capability in Capability::ALL {
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
