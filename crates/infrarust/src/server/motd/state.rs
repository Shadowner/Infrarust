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
    pub fn default_text(&self) -> String {
        match self {
            MotdState::Online => "Server is online".to_string(),
            MotdState::Offline => {
                "§e§oServer is sleeping. §8§o\nConnect to it to wake it up.".to_string()
            }
            MotdState::Starting => {
                "§6Server is starting...§r\n§8§oPlease wait a moment".to_string()
            }
            MotdState::Stopping => {
                "§6Server is marked to shutdown...\n§8§o Connect to it to cancel it !".to_string()
            }
            MotdState::ImminentShutdown { seconds_remaining } => {
                let time_str = if *seconds_remaining <= 60 {
                    format!("{} seconds", seconds_remaining)
                } else {
                    format!("{:.1} minutes", *seconds_remaining as f64 / 60.0)
                };
                format!(
                    "§c§lServer shutting down in {}!§r\n§e§oConnect now to keep it online!",
                    time_str
                )
            }
            MotdState::Crashed => {
                "§4Server is in a crashing state...§r\n§8§o -> Contact an admin if the issue persist.".to_string()
            }
            MotdState::Unreachable => {
                "§cServer is unreachable...§r\n§8§oTry again later".to_string()
            }
            MotdState::UnableToFetchStatus => {
                "§cUnable to obtain server status...§r\n§8§o -> Contact an admin if the issue persist.".to_string()
            }
            MotdState::Unknown => {
                "§cUnknown server status...§r\n§8§o -> Contact an admin if the issue persist.".to_string()
            }
            MotdState::UnknownServer => "§cServer not found".to_string(),
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
