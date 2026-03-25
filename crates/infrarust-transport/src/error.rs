//! Transport error types.

use std::net::SocketAddr;
use std::time::Duration;

/// Errors that can occur in the transport layer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// Failed to bind listener socket.
    #[error("failed to bind to {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },

    /// Failed to accept incoming connection.
    #[error("accept error: {0}")]
    Accept(std::io::Error),

    /// Failed to connect to backend server.
    #[error("failed to connect to backend {addr}: {source}")]
    BackendConnect {
        addr: SocketAddr,
        source: std::io::Error,
    },

    /// All backend addresses failed.
    #[error("all backends failed for server {server_id}")]
    AllBackendsFailed { server_id: String },

    /// Connection to backend timed out.
    #[error("connection to {addr} timed out after {timeout:?}")]
    ConnectTimeout { addr: SocketAddr, timeout: Duration },

    /// Invalid proxy protocol header.
    #[error("invalid proxy protocol: {0}")]
    InvalidProxyProtocol(String),

    /// Failed to decode proxy protocol header.
    #[error("proxy protocol decode error: {0}")]
    ProxyProtocolDecode(String),

    /// Socket configuration error.
    #[error("socket configuration error: {0}")]
    SocketConfig(std::io::Error),

    /// Forwarding I/O error.
    #[error("forward error: {0}")]
    Forward(std::io::Error),

    /// Splice syscall error (Linux only).
    #[error("splice error: {0}")]
    Splice(std::io::Error),

    /// Shutdown signal received.
    #[error("shutdown")]
    Shutdown,
}

/// Convenience type alias for transport results.
pub type TransportResult<T> = Result<T, TransportError>;
