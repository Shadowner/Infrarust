//! Network-related abstractions for protocol handling

use std::{future::Future, io};
use thiserror::Error;

use crate::packet::PacketError;

/// Error type for network protocol operations
#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("IO Error: {0}")]
    Io(#[from] io::Error),

    #[error("Packet Error: {0}")]
    Packet(#[from] PacketError),

    #[error("Invalid length: {0}")]
    InvalidLength(usize),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

/// Trait for abstracting the proxy protocol implementation
pub trait ProxyProtocol {
    fn init(&mut self) -> Result<()>;
    fn is_enabled(&self) -> bool;
}

/// Trait for abstracting a network connection with protocol capabilities
pub trait ProtocolConnection {
    type Packet;
    fn read_packet(&mut self) -> impl Future<Output = Result<Self::Packet>> + Send;
    fn write_packet(&mut self, packet: &Self::Packet) -> impl Future<Output = Result<()>> + Send;
    fn enable_encryption(&mut self, key: &[u8], iv: &[u8]) -> Result<()>;
    fn enable_compression(&mut self, threshold: i32);
    fn close(&mut self) -> impl Future<Output = Result<()>> + Send;
}
