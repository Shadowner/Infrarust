use std::io;
use thiserror::Error;

/// Represents all possible errors when handling packets
#[derive(Error, Debug)]
pub enum PacketError {
    #[error("IO Error: {0}")]
    Io(#[from] io::Error),

    #[error("Compression error: {0}")]
    Compression(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Invalid packet length: {length} (max: {max})")]
    InvalidLength { length: usize, max: usize },

    #[error("Invalid packet format: {0}")]
    InvalidFormat(String),

    #[error("Unsupported protocol version: {0}")]
    UnsupportedProtocol(i32),

    #[error("VarInt/VarLong decoding error: {0}")]
    VarIntDecoding(String),

    #[error("Invalid packet type for state {state}: {packet_id}")]
    InvalidPacketType { state: String, packet_id: i32 },

    #[error("{0}")]
    Other(String),
}

impl PacketError {
    pub fn compression(msg: impl Into<String>) -> Self {
        PacketError::Compression(msg.into())
    }

    pub fn encryption(msg: impl Into<String>) -> Self {
        PacketError::Encryption(msg.into())
    }

    pub fn invalid_format(msg: impl Into<String>) -> Self {
        PacketError::InvalidFormat(msg.into())
    }

    pub fn other(msg: impl Into<String>) -> Self {
        PacketError::Other(msg.into())
    }

    pub fn kind(&self) -> std::io::ErrorKind {
        match self {
            PacketError::Io(e) => e.kind(),
            PacketError::Compression(_) => std::io::ErrorKind::InvalidData,
            PacketError::Encryption(_) => std::io::ErrorKind::InvalidData,
            PacketError::InvalidLength { .. } => std::io::ErrorKind::InvalidInput,
            PacketError::InvalidFormat(_) => std::io::ErrorKind::InvalidData,
            PacketError::UnsupportedProtocol(_) => std::io::ErrorKind::Unsupported,
            PacketError::VarIntDecoding(_) => std::io::ErrorKind::InvalidData,
            PacketError::InvalidPacketType { .. } => std::io::ErrorKind::InvalidInput,
            PacketError::Other(_) => std::io::ErrorKind::Other,
        }
    }
}

impl From<PacketError> for std::io::Error {
    fn from(err: PacketError) -> Self {
        match err {
            PacketError::Io(e) => e,
            PacketError::Compression(msg) => {
                std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
            }
            PacketError::Encryption(msg) => {
                std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
            }
            PacketError::InvalidLength { length, max } => std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid packet length: {length} (max: {max})"),
            ),
            PacketError::InvalidFormat(msg) => {
                std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
            }
            PacketError::UnsupportedProtocol(ver) => std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Unsupported protocol version: {ver}"),
            ),
            PacketError::VarIntDecoding(msg) => {
                std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
            }
            PacketError::InvalidPacketType { state, packet_id } => std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid packet type for state {state}: {packet_id}"),
            ),
            PacketError::Other(msg) => std::io::Error::new(std::io::ErrorKind::Other, msg),
        }
    }
}

pub type PacketResult<T> = Result<T, PacketError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_messages() {
        let err = PacketError::InvalidLength {
            length: 1000,
            max: 100,
        };
        assert_eq!(err.to_string(), "Invalid packet length: 1000 (max: 100)");

        let err = PacketError::compression("Test compression");
        assert_eq!(err.to_string(), "Compression error: Test compression");

        let err = PacketError::encryption("Test encryption");
        assert_eq!(err.to_string(), "Encryption error: Test encryption");
    }

    #[test]
    fn test_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::Other, "test error");
        let packet_err: PacketError = io_err.into();
        assert!(matches!(packet_err, PacketError::Io(_)));
    }
}
