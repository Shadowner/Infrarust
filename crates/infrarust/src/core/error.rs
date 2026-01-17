
use std::{fmt, io};
use thiserror::Error;

pub use infrarust_ban_system::BanError;
pub use infrarust_protocol::network::ProtocolError;
pub use infrarust_protocol::packet::PacketError;

use crate::security::filter::FilterError;

#[derive(Debug, Error)]
pub enum InfrarustError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Packet error: {0}")]
    Packet(#[from] PacketError),

    #[error("Ban system error: {0}")]
    Ban(#[from] BanError),

    #[error("Filter error: {0}")]
    Filter(#[from] FilterError),

    #[error("RSA error: {0}")]
    Rsa(#[from] RsaError),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("{0}")]
    Other(String),
}

impl InfrarustError {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn connection(msg: impl Into<String>) -> Self {
        Self::Connection(msg.into())
    }

    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::Timeout(msg.into())
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, InfrarustError>;

#[derive(Debug)]
pub enum RsaError {
    RsaLib(rsa::Error),
    InvalidKeyLength(usize),
    KeyEncodingError(String),
    KeyGenerationError(String),
}

impl fmt::Display for RsaError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RsaError::RsaLib(err) => write!(f, "{}", err),
            RsaError::InvalidKeyLength(length) => write!(f, "Invalid key length: {}", length),
            RsaError::KeyEncodingError(msg) => write!(f, "Key encoding error: {}", msg),
            RsaError::KeyGenerationError(msg) => write!(f, "Key generation error: {}", msg),
        }
    }
}

impl std::error::Error for RsaError {}

impl From<rsa::Error> for RsaError {
    fn from(err: rsa::Error) -> Self {
        RsaError::RsaLib(err)
    }
}

impl From<RsaError> for io::Error {
    fn from(err: RsaError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, err.to_string())
    }
}

#[derive(Debug)]
pub struct InfraRustError {
    kind: InfraRustErrorKind,
    message: String,
}

#[derive(Debug)]
pub enum InfraRustErrorKind {
    Io,
    Protocol,
    Connection,
}

impl InfraRustError {
    pub fn new(kind: InfraRustErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::error::Error for InfraRustError {}

impl fmt::Display for InfraRustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl From<io::Error> for InfraRustError {
    fn from(err: io::Error) -> Self {
        Self::new(InfraRustErrorKind::Io, err.to_string())
    }
}

impl From<InfraRustError> for InfrarustError {
    fn from(err: InfraRustError) -> Self {
        InfrarustError::Other(err.to_string())
    }
}

#[derive(Debug)]
pub struct SendError {
    message: String,
}

impl SendError {
    pub fn new<E: std::error::Error>(error: E) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SendError {}

impl From<io::Error> for SendError {
    fn from(err: io::Error) -> Self {
        SendError::new(err)
    }
}

impl From<InfrarustError> for SendError {
    fn from(err: InfrarustError) -> Self {
        SendError::new(err)
    }
}
