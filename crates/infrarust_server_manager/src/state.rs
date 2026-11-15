/// State of a backend server managed by a `ServerProvider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerState {
    Online,
    /// The server is stopped, ready to be woken up.
    Sleeping,
    Starting,
    Stopping,
    Crashed,
    Unknown,
}

impl ServerState {
    pub const fn is_joinable(&self) -> bool {
        matches!(self, Self::Online)
    }

    pub const fn is_startable(&self) -> bool {
        matches!(self, Self::Sleeping | Self::Crashed)
    }

    /// Returns `true` if a player should wait for the server to start.
    pub const fn should_wait(&self) -> bool {
        matches!(self, Self::Starting)
    }
}

impl std::fmt::Display for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Online => write!(f, "Online"),
            Self::Sleeping => write!(f, "Sleeping"),
            Self::Starting => write!(f, "Starting"),
            Self::Stopping => write!(f, "Stopping"),
            Self::Crashed => write!(f, "Crashed"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}
