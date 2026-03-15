/// State of a backend server managed by a ServerProvider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerState {
    /// The server is online and accepting connections.
    Online,
    /// The server is stopped, ready to be woken up.
    Sleeping,
    /// The server is starting up.
    Starting,
    /// The server is shutting down.
    Stopping,
    /// The server crashed unexpectedly.
    Crashed,
    /// Unable to determine state.
    Unknown,
}

impl ServerState {
    /// Returns `true` if the server can accept player connections.
    pub fn is_joinable(&self) -> bool {
        matches!(self, ServerState::Online)
    }

    /// Returns `true` if the server can be started.
    pub fn is_startable(&self) -> bool {
        matches!(self, ServerState::Sleeping | ServerState::Crashed)
    }

    /// Returns `true` if a player should wait for the server to start.
    pub fn should_wait(&self) -> bool {
        matches!(self, ServerState::Starting)
    }
}

impl std::fmt::Display for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerState::Online => write!(f, "Online"),
            ServerState::Sleeping => write!(f, "Sleeping"),
            ServerState::Starting => write!(f, "Starting"),
            ServerState::Stopping => write!(f, "Stopping"),
            ServerState::Crashed => write!(f, "Crashed"),
            ServerState::Unknown => write!(f, "Unknown"),
        }
    }
}
