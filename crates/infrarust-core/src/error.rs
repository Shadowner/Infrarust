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

    #[error("connection timeout: {0}")]
    Timeout(String),

    #[error("backend unreachable: {0}")]
    BackendUnreachable(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("forwarding error: {0}")]
    Forwarding(#[from] crate::forwarding::ForwardingError),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("missing pipeline extension: {0} — check middleware ordering")]
    MissingExtension(&'static str),

    #[error("invalid provider id (expected `type@id`): {0}")]
    InvalidProviderId(String),

    #[error("docker connection error: {0}")]
    DockerConnection(String),

    #[error("telemetry initialization error: {0}")]
    TelemetryInit(String),

    #[error("{0}")]
    Other(String),
}

impl CoreError {
    /// Returns `true` if this error represents a normal disconnect
    /// (connection reset, broken pipe, EOF) that should be logged at debug level.
    pub fn is_expected_disconnect(&self) -> bool {
        match self {
            Self::Io(e) => matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::UnexpectedEof
            ),
            Self::ConnectionClosed | Self::Transport(_) => true,
            _ => false,
        }
    }
}
