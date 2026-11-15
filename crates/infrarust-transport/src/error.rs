use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    #[error("failed to bind to {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },

    #[error("socket activation error: {0}")]
    SocketActivation(std::io::Error),

    #[error("accept error: {0}")]
    Accept(std::io::Error),

    #[error("failed to connect to backend {addr}: {source}")]
    BackendConnect {
        addr: String,
        source: std::io::Error,
    },

    #[error("all backends failed for server {server_id}")]
    AllBackendsFailed { server_id: String },

    #[error("connection to {addr} timed out after {timeout:?}")]
    ConnectTimeout { addr: String, timeout: Duration },

    #[error("invalid proxy protocol: {0}")]
    InvalidProxyProtocol(String),

    #[error("proxy protocol decode error: {0}")]
    ProxyProtocolDecode(String),

    #[error("proxy protocol encode error: {0}")]
    ProxyProtocolEncode(String),

    #[error("timed out waiting for the proxy protocol header")]
    ProxyProtocolTimeout,

    #[error("proxy protocol I/O error: {0}")]
    ProxyProtocolIo(#[source] std::io::Error),

    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),

    #[error("peer {peer} is not in trusted_proxies")]
    UntrustedProxyPeer { peer: IpAddr },

    #[error("{ip} is at the per-IP connection limit")]
    IpLimitReached { ip: IpAddr },

    #[error("socket configuration error: {0}")]
    SocketConfig(std::io::Error),

    #[error("forward error: {0}")]
    Forward(std::io::Error),

    #[error("splice error: {0}")]
    Splice(std::io::Error),

    #[error("shutdown")]
    Shutdown,
}

pub type TransportResult<T> = Result<T, TransportError>;
