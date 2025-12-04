use core::fmt;
use std::io;

use crate::network::packet::PacketError;

#[derive(Debug, Clone)]
pub enum ProxyProtocolError {
    NoTrustedCIDRs,
    UpstreamNotTrusted,
    InvalidHeader(String),
    Io(String),
    InvalidLength(usize),
    VarIntTooLong(Option<String>),
    Other(String),
}

impl fmt::Display for ProxyProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTrustedCIDRs => write!(f, "No trusted CIDRs"),
            Self::UpstreamNotTrusted => write!(f, "Upstream not trusted"),
            Self::InvalidHeader(e) => write!(f, "Invalid proxy protocol header: {}", e),
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::InvalidLength(len) => write!(f, "Invalid length: {}", len),
            Self::VarIntTooLong(reason) => write!(
                f,
                "VarInt too long ({})",
                reason.clone().unwrap_or("unknown".to_string())
            ),
            Self::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ProxyProtocolError {}

impl From<io::Error> for ProxyProtocolError {
    fn from(e: io::Error) -> Self {
        ProxyProtocolError::Io(e.to_string())
    }
}

impl From<proxy_protocol::ParseError> for ProxyProtocolError {
    fn from(e: proxy_protocol::ParseError) -> Self {
        ProxyProtocolError::InvalidHeader(e.to_string())
    }
}

impl From<ProxyProtocolError> for io::Error {
    fn from(err: ProxyProtocolError) -> Self {
        io::Error::other(err.to_string())
    }
}

impl From<PacketError> for ProxyProtocolError {
    fn from(err: PacketError) -> Self {
        match err {
            PacketError::Io(e) => ProxyProtocolError::Io(e.to_string()),
            PacketError::Compression(e) => {
                ProxyProtocolError::Other(format!("Compression error: {}", e))
            }
            PacketError::Encryption(e) => {
                ProxyProtocolError::Other(format!("Encryption error: {}", e))
            }
            PacketError::InvalidLength { length, max: _ } => {
                ProxyProtocolError::InvalidLength(length)
            }
            PacketError::InvalidFormat(e) => ProxyProtocolError::InvalidHeader(e),
            PacketError::UnsupportedProtocol(version) => {
                ProxyProtocolError::Other(format!("Unsupported protocol version: {}", version))
            }
            PacketError::VarIntDecoding(e) => {
                ProxyProtocolError::VarIntTooLong(Some(e.to_string()))
            }
            PacketError::InvalidPacketType { state, packet_id } => ProxyProtocolError::Other(
                format!("Invalid packet type for state {}: {}", state, packet_id),
            ),
            PacketError::Other(e) => ProxyProtocolError::Other(e),
        }
    }
}
