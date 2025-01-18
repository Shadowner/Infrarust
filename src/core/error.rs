use std::{fmt, io};

#[derive(Debug)]
pub enum RsaError {
    RsaLib(rsa::Error),
    InvalidKeyLength(usize),
}

impl fmt::Display for RsaError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RsaError::RsaLib(err) => write!(f, "{}", err),
            RsaError::InvalidKeyLength(length) => write!(f, "Invalid key length: {}", length),
        }
    }
}

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
