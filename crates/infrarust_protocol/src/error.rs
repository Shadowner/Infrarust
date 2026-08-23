use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("incomplete: {context}")]
    Incomplete { context: &'static str },

    #[error("invalid: {context}")]
    Invalid { context: String },

    #[error("too large: {actual} bytes exceeds maximum of {max}")]
    TooLarge { max: usize, actual: usize },

    #[error(transparent)]
    Io(#[from] io::Error),
}

impl ProtocolError {
    pub fn invalid(context: impl Into<String>) -> Self {
        Self::Invalid {
            context: context.into(),
        }
    }

    pub const fn too_large(max: usize, actual: usize) -> Self {
        Self::TooLarge { max, actual }
    }

    pub const fn is_incomplete(&self) -> bool {
        matches!(self, Self::Incomplete { .. })
    }

    pub fn is_fatal(&self) -> bool {
        match self {
            Self::Incomplete { .. } => false,
            Self::Invalid { .. } | Self::TooLarge { .. } => true,
            Self::Io(err) => !matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::UnexpectedEof
            ),
        }
    }
}

pub type ProtocolResult<T> = Result<T, ProtocolError>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_incomplete_is_not_fatal() {
        let err = ProtocolError::Incomplete { context: "varint" };
        assert!(!err.is_fatal());
        assert!(err.is_incomplete());
    }

    #[test]
    fn test_invalid_is_fatal() {
        let err = ProtocolError::invalid("bad packet id");
        assert!(err.is_fatal());
        assert!(!err.is_incomplete());
    }

    #[test]
    fn test_too_large_is_fatal() {
        let err = ProtocolError::too_large(1024, 9999);
        assert!(err.is_fatal());
        assert!(!err.is_incomplete());
    }

    #[test]
    fn test_io_would_block_is_not_fatal() {
        let err = ProtocolError::Io(io::Error::new(io::ErrorKind::WouldBlock, "would block"));
        assert!(!err.is_fatal());
    }

    #[test]
    fn test_io_connection_reset_is_fatal() {
        let err = ProtocolError::Io(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
        assert!(err.is_fatal());
    }

    #[test]
    fn test_from_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broke");
        let proto_err: ProtocolError = io_err.into();
        assert!(matches!(proto_err, ProtocolError::Io(_)));
        assert!(proto_err.is_fatal());
    }

    #[test]
    fn test_display_messages_are_descriptive() {
        let incomplete = ProtocolError::Incomplete { context: "varint" };
        assert!(
            format!("{incomplete}").contains("varint"),
            "incomplete display should contain context"
        );

        let invalid = ProtocolError::invalid("bad string length");
        let msg = format!("{invalid}");
        assert!(
            msg.contains("bad string length"),
            "invalid display should contain context"
        );

        let too_large = ProtocolError::too_large(1024, 2048);
        let msg = format!("{too_large}");
        assert!(msg.contains("1024"), "too_large display should contain max");
        assert!(
            msg.contains("2048"),
            "too_large display should contain actual"
        );
    }
}
