//! Minecraft protocol implementation for Infrarust
//!
//! This crate provides the protocol implementations for Minecraft Java Edition.

pub mod minecraft;
pub mod network;
pub mod packet;
pub mod types;
pub mod version;

// Re-export the most commonly used types
pub use network::{ProtocolConnection, ProtocolError, ProxyProtocol};
pub use packet::{PacketCodec, PacketDataAccess, PacketError, PacketFactory};
pub use types::{ProtocolRead, ProtocolWrite};
