mod base;
mod builder;
mod error;
pub mod io;

// Re-export from base module
pub use base::{MAX_PACKET_DATA_LENGTH, MAX_PACKET_LENGTH, MAX_UNCOMPRESSED_LENGTH, Packet};

// Re-export from protocol crate
pub use infrarust_protocol::packet::{
    CompressionControl, CompressionState, EncryptionControl, EncryptionState, PacketCodec,
    PacketDataAccess, PacketError, PacketSerialization, PacketValidation, Result as PacketResult,
};

pub use builder::PacketBuilder;
pub use io::{PacketReader, PacketWriter};
