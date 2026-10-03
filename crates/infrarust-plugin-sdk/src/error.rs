use std::fmt;

use crate::bindings::types as wt;

pub use infrarust_plugin_common::ErrorKind;

const fn kind_from_wit(kind: wt::ErrorKind) -> ErrorKind {
    match kind {
        wt::ErrorKind::InvalidArgument => ErrorKind::InvalidArgument,
        wt::ErrorKind::NotFound => ErrorKind::NotFound,
        wt::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
        wt::ErrorKind::Unavailable => ErrorKind::Unavailable,
        wt::ErrorKind::Timeout => ErrorKind::Timeout,
        wt::ErrorKind::PlayerGone => ErrorKind::PlayerGone,
        wt::ErrorKind::Conflict => ErrorKind::Conflict,
        wt::ErrorKind::InvalidState => ErrorKind::InvalidState,
        wt::ErrorKind::Unsupported => ErrorKind::Unsupported,
        wt::ErrorKind::Internal => ErrorKind::Internal,
        wt::ErrorKind::LimitExceeded => ErrorKind::LimitExceeded,
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
            kind: kind_from_wit(error.kind),
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
    fn a_quota_refusal_reads_as_limit_exceeded() {
        let error = Error::from(wt::HostError {
            kind: wt::ErrorKind::LimitExceeded,
            message: "the plugin already holds 256 commands".into(),
        });
        assert_eq!(error.kind(), ErrorKind::LimitExceeded);
        assert_eq!(
            error.to_string(),
            "limit-exceeded: the plugin already holds 256 commands"
        );
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
