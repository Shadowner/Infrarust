//! Error types for the forwarding module.

#[derive(Debug, thiserror::Error)]
pub enum ForwardingError {
    #[error("backend did not send login plugin request for velocity:player_info")]
    NoVelocityRequest,

    #[error("HMAC-SHA256 verification failed")]
    InvalidSignature,

    #[error("unsupported velocity forwarding version: {0}")]
    UnsupportedVersion(u8),

    #[error("I/O error during forwarding: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error during forwarding: {0}")]
    Protocol(#[from] infrarust_protocol::ProtocolError),

    #[error("backend disconnected during forwarding")]
    Disconnected,

    #[error("backend rejected connection during forwarding: {0}")]
    Rejected(String),
}
