//! Error types for the Infrarust plugin API.

pub use infrarust_plugin_common::ErrorKind;

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

    #[error("player is not connected to a backend server")]
    NoBackend,

    #[error("plugin message of {size} bytes is over the {max} bytes allowed")]
    MessageTooLarge { size: usize, max: usize },

    #[error("not supported: {0}")]
    Unsupported(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("denied: {0}")]
    Denied(Box<crate::types::Component>),

    #[error(
        "the player's session is waiting for the code that made this call, so it cannot answer it; use switch_server, or await the call in a task of its own"
    )]
    WouldDeadlock,
}

impl PlayerError {
    /// The [`ErrorKind`] a WASM guest receives for this error.
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Disconnected => ErrorKind::PlayerGone,
            Self::NotActive | Self::NoBackend | Self::WouldDeadlock => ErrorKind::InvalidState,
            Self::ServerNotFound(_) => ErrorKind::NotFound,
            Self::MessageTooLarge { .. } | Self::InvalidArgument(_) => ErrorKind::InvalidArgument,
            Self::SendFailed(_) | Self::SwitchFailed(_) => ErrorKind::Unavailable,
            Self::Unsupported(_) => ErrorKind::Unsupported,
            Self::Denied(_) => ErrorKind::PermissionDenied,
        }
    }
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

    #[error("operation failed: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// The service is temporarily unavailable.
    #[error("service unavailable: {0}")]
    Unavailable(String),

    #[error("service `{service}` is already provided by `{by}`")]
    AlreadyProvided { service: &'static str, by: String },
}

impl ServiceError {
    /// The [`ErrorKind`] a WASM guest receives for this error.
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::NotFound(_) => ErrorKind::NotFound,
            Self::Unavailable(_) => ErrorKind::Unavailable,
            Self::AlreadyProvided { .. } => ErrorKind::Conflict,
            Self::OperationFailed(_) | Self::Internal(_) => ErrorKind::Internal,
        }
    }
}

/// Errors that can occur during plugin lifecycle.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// Plugin initialization failed.
    #[error("plugin initialization failed: {0}")]
    InitFailed(String),

    #[error(transparent)]
    Service(#[from] ServiceError),

    #[error(transparent)]
    Limbo(#[from] crate::limbo::LimboHandlerError),

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl From<String> for PluginError {
    fn from(s: String) -> Self {
        Self::Other(s.into())
    }
}

impl From<&str> for PluginError {
    fn from(s: &str) -> Self {
        Self::Other(s.into())
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
        assert!(matches!(err, PluginError::Other(_)));
        assert_eq!(err.to_string(), "something went wrong");
    }

    #[test]
    fn plugin_error_from_owned_string() {
        let err: PluginError = String::from("failure").into();
        assert!(matches!(err, PluginError::Other(_)));
    }

    #[test]
    fn plugin_error_keeps_the_service_and_limbo_errors_it_wraps() {
        let service: PluginError = ServiceError::NotFound("lobby".into()).into();
        assert!(matches!(
            service,
            PluginError::Service(ServiceError::NotFound(ref id)) if id == "lobby"
        ));
        assert_eq!(service.to_string(), "not found: lobby");

        let limbo: PluginError = crate::limbo::LimboHandlerError::MissingCapability.into();
        assert!(matches!(
            limbo,
            PluginError::Limbo(crate::limbo::LimboHandlerError::MissingCapability)
        ));
        assert!(std::error::Error::source(&limbo).is_none());

        let io: PluginError =
            Box::<dyn std::error::Error + Send + Sync>::from(std::io::Error::other("disk")).into();
        assert!(matches!(io, PluginError::Other(_)));
        assert_eq!(io.to_string(), "disk");
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
