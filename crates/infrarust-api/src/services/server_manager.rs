//! Server manager service.

use crate::error::ServiceError;
use crate::event::{BoxFuture, ListenerHandle};
use crate::types::ServerId;

mod private {
    /// Sealed — only the proxy implements [`ServerManager`](super::ServerManager).
    pub trait Sealed {}
}

/// The state of a backend server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerState {
    /// Server is online and accepting connections.
    Online,
    /// Server is offline.
    Offline,
    /// Server is in the process of starting.
    Starting,
    /// Server is in the process of stopping.
    Stopping,
    /// Server is in sleep mode (can be woken on demand).
    Sleeping,
    /// Server has crashed.
    Crashed,
}

/// A callback for server state change notifications.
pub type StateChangeCallback = Box<dyn Fn(&ServerId, ServerState, ServerState) + Send + Sync>;

/// Service for managing backend server lifecycle.
///
/// Obtained via [`PluginContext::server_manager()`](crate::plugin::PluginContext::server_manager).
pub trait ServerManager: Send + Sync + private::Sealed {
    /// Returns the current state of a server, or `None` if the server ID is unknown.
    fn get_state(&self, server: &ServerId) -> Option<ServerState>;

    /// Starts a server. Returns an error if the server is already running
    /// or the ID is unknown.
    fn start(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>>;

    /// Stops a server. Returns an error if the server is not running
    /// or the ID is unknown.
    fn stop(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>>;

    /// Registers a callback for server state changes.
    ///
    /// The callback receives the server ID, old state, and new state.
    fn on_state_change(&self, callback: StateChangeCallback) -> ListenerHandle;

    /// Returns all servers and their current states.
    fn get_all_servers(&self) -> Vec<(ServerId, ServerState)>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_state_non_exhaustive() {
        let state = ServerState::Online;
        #[allow(unreachable_patterns)]
        match state {
            ServerState::Online
            | ServerState::Offline
            | ServerState::Starting
            | ServerState::Stopping
            | ServerState::Sleeping
            | ServerState::Crashed
            | _ => {}
        }
    }
}
