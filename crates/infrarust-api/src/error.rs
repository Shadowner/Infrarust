//! Error types for the Infrarust plugin API.

/// Errors that can occur when interacting with a player.
///
/// Returned by [`Player`](crate::player::Player) methods when an operation
/// cannot be completed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlayerError {
    /// The player is not on an active proxy path (e.g. passthrough/zero-copy mode).
    #[error("player is not active — operation requires an active proxy connection")]
    NotActive,

    /// The player has already disconnected from the proxy.
    #[error("player is disconnected")]
    Disconnected,

    /// Failed to send a packet or message to the player.
    #[error("send failed: {0}")]
    SendFailed(String),

    /// The target server does not exist in the proxy configuration.
    #[error("server not found: {0}")]
    ServerNotFound(String),

    /// A server switch operation failed.
    #[error("switch failed: {0}")]
    SwitchFailed(String),
}

/// Errors that can occur when interacting with proxy services.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// The requested resource was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// The operation failed.
    #[error("operation failed: {0}")]
    OperationFailed(String),

    /// The service is temporarily unavailable.
    #[error("service unavailable: {0}")]
    Unavailable(String),
}

/// Errors that can occur during plugin lifecycle.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// Plugin initialization failed.
    #[error("plugin initialization failed: {0}")]
    InitFailed(String),

    /// A custom plugin error.
    #[error("{0}")]
    Custom(String),
}

impl From<String> for PluginError {
    fn from(s: String) -> Self {
        Self::Custom(s)
    }
}

impl From<&str> for PluginError {
    fn from(s: &str) -> Self {
        Self::Custom(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn player_error_display() {
        let err = PlayerError::NotActive;
        assert!(err.to_string().contains("not active"));

        let err = PlayerError::SendFailed("timeout".into());
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn service_error_display() {
        let err = ServiceError::NotFound("lobby".into());
        assert!(err.to_string().contains("lobby"));
    }

    #[test]
    fn plugin_error_from_string() {
        let err: PluginError = "something went wrong".into();
        assert!(matches!(err, PluginError::Custom(_)));
        assert!(err.to_string().contains("something went wrong"));
    }

    #[test]
    fn plugin_error_from_owned_string() {
        let err: PluginError = String::from("failure").into();
        assert!(matches!(err, PluginError::Custom(_)));
    }

    #[test]
    fn non_exhaustive_match() {
        let err = PlayerError::Disconnected;
        #[allow(unreachable_patterns)]
        match err {
            PlayerError::NotActive
            | PlayerError::Disconnected
            | PlayerError::SendFailed(_)
            | PlayerError::ServerNotFound(_)
            | PlayerError::SwitchFailed(_)
            | _ => {}
        }
    }
}
