use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotdState {
    /// Server is online and running normally
    Online,
    /// Server is offline/stopped and can be woken up
    Offline,
    /// Server is currently starting up
    Starting,
    /// Server is marked for shutdown but still running
    Stopping,
    /// Server is about to shut down (with countdown)
    ImminentShutdown { seconds_remaining: u64 },
    /// Server has crashed
    Crashed,
    /// Server is unreachable
    Unreachable,
    /// Unable to obtain server status
    UnableToFetchStatus,
    /// Server status is unknown
    Unknown,
    /// Server domain is not configured
    UnknownServer,
}

impl MotdState {
    pub fn default_text(&self) -> Cow<'static, str> {
        match self {
            MotdState::Online => Cow::Borrowed("Server is online"),
            MotdState::Offline => {
                Cow::Borrowed("§e§oServer is sleeping. §8§o\nConnect to it to wake it up.")
            }
            MotdState::Starting => {
                Cow::Borrowed("§6Server is starting...§r\n§8§oPlease wait a moment")
            }
            MotdState::Stopping => Cow::Borrowed(
                "§6Server is marked to shutdown...\n§8§o Connect to it to cancel it !",
            ),
            MotdState::ImminentShutdown { seconds_remaining } => {
                let time_str = if *seconds_remaining <= 60 {
                    format!("{} seconds", seconds_remaining)
                } else {
                    format!("{:.1} minutes", *seconds_remaining as f64 / 60.0)
                };
                Cow::Owned(format!(
                    "§c§lServer shutting down in {}!§r\n§e§oConnect now to keep it online!",
                    time_str
                ))
            }
            MotdState::Crashed => Cow::Borrowed(
                "§4Server is in a crashing state...§r\n§8§o -> Contact an admin if the issue persist.",
            ),
            MotdState::Unreachable => {
                Cow::Borrowed("§cServer is unreachable...§r\n§8§oTry again later")
            }
            MotdState::UnableToFetchStatus => Cow::Borrowed(
                "§cUnable to obtain server status...§r\n§8§o -> Contact an admin if the issue persist.",
            ),
            MotdState::Unknown => Cow::Borrowed(
                "§cUnknown server status...§r\n§8§o -> Contact an admin if the issue persist.",
            ),
            MotdState::UnknownServer => Cow::Borrowed("§cServer not found"),
        }
    }

    pub fn use_default_favicon(&self) -> bool {
        match self {
            MotdState::Offline
            | MotdState::Starting
            | MotdState::Online
            | MotdState::UnknownServer => true,
            MotdState::Stopping
            | MotdState::ImminentShutdown { .. }
            | MotdState::Crashed
            | MotdState::Unreachable
            | MotdState::UnableToFetchStatus
            | MotdState::Unknown => false,
        }
    }
}
