#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerState {
    Online,
    Offline,
    Starting,
    Stopping,
    Sleeping,
    Crashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProxyMode {
    Passthrough,
    ZeroCopy,
    ClientOnly,
    Offline,
    ServerOnly,
}

impl ProxyMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passthrough => "passthrough",
            Self::ZeroCopy => "zero_copy",
            Self::ClientOnly => "client_only",
            Self::Offline => "offline",
            Self::ServerOnly => "server_only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PacketDirection {
    Serverbound,
    Clientbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnknownDomainBehavior {
    DefaultMotd,
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessagePhase {
    Configuration,
    Play,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ChatMode {
    #[default]
    Enabled,
    CommandsOnly,
    Hidden,
}

impl ChatMode {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::CommandsOnly,
            2 => Self::Hidden,
            _ => Self::Enabled,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::Enabled => 0,
            Self::CommandsOnly => 1,
            Self::Hidden => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MainHand {
    Left,
    #[default]
    Right,
}

impl MainHand {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        if id == 0 { Self::Left } else { Self::Right }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ParticleStatus {
    #[default]
    All,
    Decreased,
    Minimal,
}

impl ParticleStatus {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Decreased,
            2 => Self::Minimal,
            _ => Self::All,
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::All => 0,
            Self::Decreased => 1,
            Self::Minimal => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HandshakeIntent {
    Status,
    Login,
    Transfer,
}

impl HandshakeIntent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Login => "login",
            Self::Transfer => "transfer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConnectCause {
    Initial,
    Switch,
    LimboExit,
    KickRedirect,
    PluginMessage,
}

impl ConnectCause {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Switch => "switch",
            Self::LimboExit => "limbo_exit",
            Self::KickRedirect => "kick_redirect",
            Self::PluginMessage => "plugin_message",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransferOrigin {
    Plugin,
    Backend,
}

impl TransferOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plugin => "plugin",
            Self::Backend => "backend",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LoginStage {
    Status,
    PreAuth,
    PostAuth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionEndReason {
    Disconnected,
    Released,
    Kicked,
    Redirected,
    TimedOut,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BackendState {
    Healthy,
    Probing,
    Unhealthy,
    Draining,
}

impl BackendState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Probing => "probing",
            Self::Unhealthy => "unhealthy",
            Self::Draining => "draining",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResourcePackStatus {
    SuccessfullyLoaded,
    Declined,
    FailedDownload,
    Accepted,
    Downloaded,
    InvalidUrl,
    FailedReload,
    Discarded,
    Unknown(i32),
}

impl ResourcePackStatus {
    #[must_use]
    pub const fn from_id(id: i32) -> Self {
        match id {
            0 => Self::SuccessfullyLoaded,
            1 => Self::Declined,
            2 => Self::FailedDownload,
            3 => Self::Accepted,
            4 => Self::Downloaded,
            5 => Self::InvalidUrl,
            6 => Self::FailedReload,
            7 => Self::Discarded,
            other => Self::Unknown(other),
        }
    }

    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::SuccessfullyLoaded => 0,
            Self::Declined => 1,
            Self::FailedDownload => 2,
            Self::Accepted => 3,
            Self::Downloaded => 4,
            Self::InvalidUrl => 5,
            Self::FailedReload => 6,
            Self::Discarded => 7,
            Self::Unknown(id) => id,
        }
    }

    #[must_use]
    pub const fn is_final(self) -> bool {
        !matches!(self, Self::Accepted | Self::Downloaded | Self::Unknown(_))
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SuccessfullyLoaded => "successfully_loaded",
            Self::Declined => "declined",
            Self::FailedDownload => "failed_download",
            Self::Accepted => "accepted",
            Self::Downloaded => "downloaded",
            Self::InvalidUrl => "invalid_url",
            Self::FailedReload => "failed_reload",
            Self::Discarded => "discarded",
            Self::Unknown(_) => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilterPriority {
    First = 0,
    Early = 1,
    #[default]
    Normal = 2,
    Late = 3,
    Last = 4,
}

/// Order in which event listeners run, from [`FIRST`](Self::FIRST) to
/// [`LAST`](Self::LAST).
///
/// Each listener sees the changes made by the listeners that ran before it.
/// Two priorities are equal when their values are equal, so
/// `EventPriority::custom(128) == EventPriority::NORMAL`.
///
/// ```
/// use infrarust_plugin_common::EventPriority;
///
/// assert!(EventPriority::FIRST < EventPriority::EARLY);
/// assert_eq!(EventPriority::custom(128), EventPriority::NORMAL);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventPriority(u8);

impl EventPriority {
    /// Runs before every other listener.
    pub const FIRST: Self = Self(0);
    /// Runs before the normal listeners.
    pub const EARLY: Self = Self(64);
    /// The default priority.
    pub const NORMAL: Self = Self(128);
    /// Runs after the normal listeners.
    pub const LATE: Self = Self(192);
    /// Runs after every other listener.
    pub const LAST: Self = Self(255);

    /// A priority between the named levels; lower values run first.
    #[must_use]
    pub const fn custom(value: u8) -> Self {
        Self(value)
    }

    /// The raw value, `0` for [`FIRST`](Self::FIRST) up to `255` for [`LAST`](Self::LAST).
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn proxy_modes_are_named_as_the_config_spells_them() {
        assert_eq!(ProxyMode::Passthrough.as_str(), "passthrough");
        assert_eq!(ProxyMode::ZeroCopy.as_str(), "zero_copy");
        assert_eq!(ProxyMode::ClientOnly.as_str(), "client_only");
        assert_eq!(ProxyMode::Offline.as_str(), "offline");
        assert_eq!(ProxyMode::ServerOnly.as_str(), "server_only");
    }

    #[test]
    fn client_setting_ids_round_trip() {
        for id in 0..3 {
            assert_eq!(ChatMode::from_id(id).id(), id);
            assert_eq!(ParticleStatus::from_id(id).id(), id);
        }
        for id in 0..2 {
            assert_eq!(MainHand::from_id(id).id(), id);
        }
        assert_eq!(ChatMode::from_id(9), ChatMode::Enabled);
        assert_eq!(MainHand::from_id(9), MainHand::Right);
        assert_eq!(ParticleStatus::from_id(9), ParticleStatus::All);
        assert_eq!(ChatMode::default(), ChatMode::Enabled);
        assert_eq!(MainHand::default(), MainHand::Right);
        assert_eq!(ParticleStatus::default(), ParticleStatus::All);
    }

    #[test]
    fn resource_pack_statuses_keep_their_ids_and_finality() {
        for id in 0..8 {
            let status = ResourcePackStatus::from_id(id);
            assert_eq!(status.id(), id);
            assert_ne!(status.as_str(), "unknown");
        }
        assert_eq!(
            ResourcePackStatus::from_id(42),
            ResourcePackStatus::Unknown(42)
        );
        assert_eq!(ResourcePackStatus::Unknown(42).id(), 42);
        assert!(!ResourcePackStatus::Accepted.is_final());
        assert!(!ResourcePackStatus::Downloaded.is_final());
        assert!(!ResourcePackStatus::Unknown(9).is_final());
        assert!(ResourcePackStatus::Declined.is_final());
        assert!(ResourcePackStatus::SuccessfullyLoaded.is_final());
    }

    #[test]
    fn filter_priorities_order_first_to_last() {
        assert!(FilterPriority::First < FilterPriority::Early);
        assert!(FilterPriority::Early < FilterPriority::Normal);
        assert!(FilterPriority::Normal < FilterPriority::Late);
        assert!(FilterPriority::Late < FilterPriority::Last);
        assert_eq!(FilterPriority::default(), FilterPriority::Normal);
        assert_eq!(FilterPriority::Last as u8, 4);
    }

    #[test]
    fn event_priorities_compare_by_value() {
        assert!(EventPriority::FIRST < EventPriority::EARLY);
        assert!(EventPriority::EARLY < EventPriority::NORMAL);
        assert!(EventPriority::NORMAL < EventPriority::LATE);
        assert!(EventPriority::LATE < EventPriority::LAST);
        assert_eq!(EventPriority::custom(128), EventPriority::NORMAL);
        assert_eq!(EventPriority::LAST.value(), 255);
    }

    #[test]
    fn names_are_snake_case() {
        assert_eq!(HandshakeIntent::Transfer.as_str(), "transfer");
        assert_eq!(ConnectCause::KickRedirect.as_str(), "kick_redirect");
        assert_eq!(TransferOrigin::Backend.as_str(), "backend");
        assert_eq!(BackendState::Unhealthy.as_str(), "unhealthy");
    }
}
