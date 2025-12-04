//! Abstract packet types and traits for working with Minecraft protocol and old V1 code

use bytes::BytesMut;
use std::io;
use thiserror::Error;

use crate::version::Version;

pub mod mock;

/// Error type for packet handling operations
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

impl From<PacketError> for io::Error {
    fn from(err: PacketError) -> Self {
        match err {
            PacketError::Io(e) => e,
            PacketError::Compression(msg) => io::Error::new(io::ErrorKind::InvalidData, msg),
            PacketError::Encryption(msg) => io::Error::new(io::ErrorKind::InvalidData, msg),
            PacketError::InvalidLength { .. } => {
                io::Error::new(io::ErrorKind::InvalidInput, err.to_string())
            }
            PacketError::InvalidFormat(msg) => io::Error::new(io::ErrorKind::InvalidData, msg),
            PacketError::UnsupportedProtocol(_) => {
                io::Error::new(io::ErrorKind::Unsupported, err.to_string())
            }
            PacketError::VarIntDecoding(msg) => io::Error::new(io::ErrorKind::InvalidData, msg),
            PacketError::InvalidPacketType { .. } => {
                io::Error::new(io::ErrorKind::InvalidInput, err.to_string())
            }
            PacketError::Other(msg) => io::Error::other(msg),
        }
    }
}

impl PacketError {
    /// Returns the kind of error, similar to how std::io::Error works
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            PacketError::Io(e) => e.kind(),
            PacketError::Compression(_) => io::ErrorKind::InvalidData,
            PacketError::Encryption(_) => io::ErrorKind::InvalidData,
            PacketError::InvalidLength { .. } => io::ErrorKind::InvalidInput,
            PacketError::InvalidFormat(_) => io::ErrorKind::InvalidData,
            PacketError::UnsupportedProtocol(_) => io::ErrorKind::Unsupported,
            PacketError::VarIntDecoding(_) => io::ErrorKind::InvalidData,
            PacketError::InvalidPacketType { .. } => io::ErrorKind::InvalidInput,
            PacketError::Other(_) => io::ErrorKind::Other,
        }
    }
}

pub type Result<T> = std::result::Result<T, PacketError>;

/// Trait defining compression states for packets
pub trait CompressionControl {
    fn compression_state(&self) -> CompressionState;
    fn enable_compression(&mut self, threshold: i32);
    fn disable_compression(&mut self);
    fn is_compressing(&self) -> bool;
}

/// Trait defining encryption capabilities for packets
pub trait EncryptionControl {
    fn encryption_state(&self) -> EncryptionState;
    fn enable_encryption(&mut self);
    fn disable_encryption(&mut self);
    fn mark_as_encrypted(&mut self);
    fn is_encrypted(&self) -> bool;
}

/// Compression state for a packet
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompressionState {
    Disabled,
    Enabled { threshold: i32 },
}

/// Encryption state for a packet
#[derive(Debug, Clone, PartialEq)]
pub enum EncryptionState {
    Disabled,
    Enabled { encrypted_data: bool },
}

pub trait PacketValidation {
    fn validate_length(&self) -> Result<()>;
    fn validate_encryption(&self) -> Result<()>;
    fn validate_compression(&self) -> Result<()>;

    fn validate(&self) -> Result<()> {
        self.validate_length()?;
        self.validate_encryption()?;
        self.validate_compression()?;
        Ok(())
    }
}

pub trait PacketDataAccess {
    fn id(&self) -> i32;
    fn data(&self) -> &[u8];
    fn protocol_version(&self) -> Version;
    fn set_protocol_version(&mut self, version: Version);
}

pub trait PacketCodec: PacketDataAccess {
    fn encode<T: crate::types::ProtocolWrite>(&mut self, value: &T) -> Result<()>;
    fn decode<T: crate::types::ProtocolRead>(&self) -> Result<T>;
}

pub trait PacketFactory {
    /// The packet type this factory creates
    type Packet: PacketCodec + PacketValidation + CompressionControl + EncryptionControl;
    fn create_packet(&self, id: i32) -> Self::Packet;
    fn create_from_bytes(&self, bytes: BytesMut) -> Result<Self::Packet>;
}

pub trait PacketSerialization {
    fn into_raw_bytes(self) -> Result<BytesMut>;
    fn from_raw_bytes(bytes: BytesMut) -> Result<Self>
    where
        Self: Sized;
}

// Re-export the mock implementations for convenience
pub use mock::{MockPacket, MockPacketFactory};
