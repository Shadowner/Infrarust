/// Core error types for the infrarust-core crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("transport error: {0}")]
    Transport(#[from] infrarust_transport::TransportError),

    #[error("protocol error: {0}")]
    Protocol(#[from] infrarust_protocol::ProtocolError),

    #[error("config error: {0}")]
    Config(#[from] infrarust_config::ConfigError),

    #[error("pipeline rejected: {0}")]
    Rejected(String),

    #[error("no server found for domain: {0}")]
    UnknownDomain(String),

    #[error("connection closed")]
    ConnectionClosed,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("missing pipeline extension: {0} — check middleware ordering")]
    MissingExtension(&'static str),

    #[error("invalid provider id (expected `type@id`): {0}")]
    InvalidProviderId(String),

    #[error("docker connection error: {0}")]
    DockerConnection(String),

    #[error("{0}")]
    Other(String),
}
