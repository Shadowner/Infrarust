//! Server manager service.

use crate::error::ServiceError;
use crate::event::BoxFuture;
use crate::types::ServerId;

pub mod private {
    /// Sealed — only the proxy implements [`ServerManager`](super::ServerManager).
    pub trait Sealed {}
}

pub use infrarust_plugin_common::enums::ServerState;

/// Service for managing backend server lifecycle.
///
/// Obtained via [`PluginContext::server_manager()`](crate::plugin::PluginContext::server_manager).
pub trait ServerManager: Send + Sync + private::Sealed {
    /// Returns the current state of a server, or `None` if the server ID is unknown.
    fn get_state(&self, server: &ServerId) -> Option<ServerState>;

    /// Starts a server. Returns an error if the server is already running
    /// or the ID is unknown.
    fn start(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>>;

    /// Starts a server for a player who is trying to join it.
    ///
    /// Unlike [`start`](Self::start), this obeys the proxy's start throttle: it returns
    /// [`ServiceError::Unavailable`] when the server was started or crashed too
    /// recently. A server that is already online or starting is left as it is.
    fn wake(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>>;

    /// Stops a server. Returns an error if the server is not running
    /// or the ID is unknown.
    fn stop(&self, server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>>;

    /// Returns all servers and their current states.
    fn get_all_servers(&self) -> Vec<(ServerId, ServerState)>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
