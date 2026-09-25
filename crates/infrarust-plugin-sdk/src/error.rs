use std::fmt;

use crate::bindings::types as wt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    InvalidArgument,
    NotFound,
    PermissionDenied,
    Unavailable,
    Timeout,
    PlayerGone,
    Conflict,
    InvalidState,
    Unsupported,
    Internal,
}

impl ErrorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid-argument",
            Self::NotFound => "not-found",
            Self::PermissionDenied => "permission-denied",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::PlayerGone => "player-gone",
            Self::Conflict => "conflict",
            Self::InvalidState => "invalid-state",
            Self::Unsupported => "unsupported",
            Self::Internal => "internal",
        }
    }

    pub(crate) const fn from_wit(kind: wt::ErrorKind) -> Self {
        match kind {
            wt::ErrorKind::InvalidArgument => Self::InvalidArgument,
            wt::ErrorKind::NotFound => Self::NotFound,
            wt::ErrorKind::PermissionDenied => Self::PermissionDenied,
            wt::ErrorKind::Unavailable => Self::Unavailable,
            wt::ErrorKind::Timeout => Self::Timeout,
            wt::ErrorKind::PlayerGone => Self::PlayerGone,
            wt::ErrorKind::Conflict => Self::Conflict,
            wt::ErrorKind::InvalidState => Self::InvalidState,
            wt::ErrorKind::Unsupported => Self::Unsupported,
            wt::ErrorKind::Internal => Self::Internal,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for Error {}

impl From<wt::HostError> for Error {
    fn from(error: wt::HostError) -> Self {
        Self {
            kind: ErrorKind::from_wit(error.kind),
            message: error.message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginError {
    message: String,
}

impl PluginError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PluginError {}

impl From<String> for PluginError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for PluginError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<Error> for PluginError {
    fn from(error: Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<std::io::Error> for PluginError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<PluginError> for String {
    fn from(error: PluginError) -> Self {
        error.message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fails() -> Result<(), Error> {
        Err(Error::new(
            ErrorKind::PermissionDenied,
            "missing capability: ban",
        ))
    }

    fn enable() -> Result<(), PluginError> {
        fails()?;
        Ok(())
    }

    #[test]
    fn a_host_error_keeps_its_kind_and_message() {
        let error = Error::from(wt::HostError {
            kind: wt::ErrorKind::PlayerGone,
            message: "player 7 is not online".into(),
        });
        assert_eq!(error.kind(), ErrorKind::PlayerGone);
        assert_eq!(error.to_string(), "player-gone: player 7 is not online");
    }

    #[test]
    fn question_mark_turns_an_error_into_a_plugin_error() {
        let error = enable().unwrap_err();
        assert_eq!(
            error.message(),
            "permission-denied: missing capability: ban"
        );
        assert_eq!(PluginError::from("boom").to_string(), "boom");
        assert_eq!(String::from(PluginError::from("x".to_owned())), "x");
    }
}
