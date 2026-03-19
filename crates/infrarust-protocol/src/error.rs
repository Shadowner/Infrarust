//! Protocol error types for the Infrarust proxy.
//!
//! Provides a single unified error type [`ProtocolError`] that categorizes errors
//! into actionable categories: incomplete data (retry), invalid data (close connection),
//! oversized data (potential attack), and underlying I/O errors.

use std::io;

use thiserror::Error;

/// Unified error type for all protocol operations.
///
/// Designed to let callers distinguish between recoverable and fatal errors:
/// - [`Incomplete`](ProtocolError::Incomplete) — not enough data yet, wait and retry.
/// - [`Invalid`](ProtocolError::Invalid) — corrupted or unexpected data, close the connection.
/// - [`TooLarge`](ProtocolError::TooLarge) — size exceeds limits, potential attack.
/// - [`Io`](ProtocolError::Io) — underlying I/O error.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Not enough data to complete decoding.
    /// Non-fatal: the caller should wait for more bytes and retry.
    #[error("incomplete: {context}")]
    Incomplete {
        /// Static description of what was being decoded.
        context: &'static str,
    },

    /// Invalid or corrupted data. The connection should be closed.
    #[error("invalid: {context}")]
    Invalid {
        /// Description of what was invalid.
        context: String,
    },

    /// Size exceeds a protocol limit. Potential attack vector.
    #[error("too large: {actual} bytes exceeds maximum of {max}")]
    TooLarge {
        max: usize,
        actual: usize,
    },

    /// Underlying I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl ProtocolError {
    /// Creates an [`Invalid`](ProtocolError::Invalid) error with the given context.
    pub fn invalid(context: impl Into<String>) -> Self {
        Self::Invalid {
            context: context.into(),
        }
    }

    /// Creates a [`TooLarge`](ProtocolError::TooLarge) error.
    pub const fn too_large(max: usize, actual: usize) -> Self {
        Self::TooLarge { max, actual }
    }

    /// Returns `true` if this error indicates incomplete data (non-fatal).
    pub const fn is_incomplete(&self) -> bool {
        matches!(self, Self::Incomplete { .. })
    }

    /// Returns `true` if this error is fatal and the connection should be closed.
    ///
    /// - `Incomplete` is non-fatal (wait for more data).
    /// - `Invalid` and `TooLarge` are always fatal.
    /// - `Io` depends on the error kind: `WouldBlock` and `UnexpectedEof` are non-fatal,
    ///   everything else is fatal.
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

/// Convenience result type for protocol operations.
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
