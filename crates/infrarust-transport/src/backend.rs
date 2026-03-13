//! Backend connection management.
//!
//! `BackendConnector` handles connecting to backend Minecraft servers
//! with failover, timeout, and optional proxy protocol v2.

use std::net::SocketAddr;
use std::time::Duration;

use socket2::Socket;
use tokio::net::TcpStream;
use tokio::time::Instant;

use infrarust_config::{KeepaliveConfig, ServerAddress};

use crate::connection::ConnectionInfo;
use crate::error::TransportError;
use crate::proxy_protocol::encode_proxy_protocol_v2;
use crate::socket::configure_stream_socket;

/// Connects to backend servers with failover and timeout.
#[derive(Debug, Clone)]
pub struct BackendConnector {
    /// Default connection timeout.
    pub default_timeout: Duration,
    /// TCP keepalive configuration for backend connections.
    pub keepalive: KeepaliveConfig,
}

impl BackendConnector {
    /// Creates a new backend connector.
    pub fn new(default_timeout: Duration, keepalive: KeepaliveConfig) -> Self {
        Self {
            default_timeout,
            keepalive,
        }
    }

    /// Connects to one of the given backend addresses.
    ///
    /// Tries each address in order. On the first successful connection,
    /// configures the socket (TCP_NODELAY, keepalive) and optionally
    /// sends a proxy protocol v2 header.
    ///
    /// Returns `AllBackendsFailed` if no address could be reached.
    pub async fn connect(
        &self,
        server_id: &str,
        addresses: &[ServerAddress],
        timeout_override: Option<Duration>,
        send_proxy_protocol: bool,
        client_info: &ConnectionInfo,
    ) -> Result<BackendConnection, TransportError> {
        let timeout = timeout_override.unwrap_or(self.default_timeout);
        let mut last_error = None;

        for address in addresses {
            match self
                .try_connect(address, timeout, send_proxy_protocol, client_info)
                .await
            {
                Ok(conn) => return Ok(conn),
                Err(e) => {
                    tracing::warn!(
                        server_id = server_id,
                        address = %address,
                        error = %e,
                        "backend connection attempt failed"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or(TransportError::AllBackendsFailed {
            server_id: server_id.to_string(),
        }))
    }

    async fn try_connect(
        &self,
        address: &ServerAddress,
        timeout: Duration,
        send_proxy_protocol: bool,
        client_info: &ConnectionInfo,
    ) -> Result<BackendConnection, TransportError> {
        let connect_addr = format!("{}:{}", address.host, address.port);

        let mut stream = tokio::time::timeout(timeout, TcpStream::connect(&connect_addr))
            .await
            .map_err(|_| {
                let addr: SocketAddr = connect_addr
                    .parse()
                    .unwrap_or_else(|_| SocketAddr::new([0, 0, 0, 0].into(), address.port));
                TransportError::ConnectTimeout { addr, timeout }
            })?
            .map_err(|source| {
                let addr: SocketAddr = connect_addr
                    .parse()
                    .unwrap_or_else(|_| SocketAddr::new([0, 0, 0, 0].into(), address.port));
                TransportError::BackendConnect { addr, source }
            })?;

        let remote_addr = stream.peer_addr().map_err(TransportError::SocketConfig)?;

        // Configure socket via socket2 round-trip
        let std_stream = stream.into_std().map_err(TransportError::SocketConfig)?;
        let socket = Socket::from(std_stream);
        configure_stream_socket(&socket, &self.keepalive)?;
        socket
            .set_nonblocking(true)
            .map_err(TransportError::SocketConfig)?;
        let std_stream: std::net::TcpStream = socket.into();
        stream = TcpStream::from_std(std_stream).map_err(TransportError::SocketConfig)?;

        if send_proxy_protocol {
            encode_proxy_protocol_v2(&mut stream, client_info).await?;
        }

        Ok(BackendConnection {
            stream,
            remote_addr,
            connected_at: Instant::now(),
        })
    }
}

/// An established connection to a backend server.
#[derive(Debug)]
pub struct BackendConnection {
    stream: TcpStream,
    remote_addr: SocketAddr,
    connected_at: Instant,
}

impl BackendConnection {
    /// Consumes the connection and returns the TCP stream.
    pub fn into_stream(self) -> TcpStream {
        self.stream
    }

    /// Returns a reference to the TCP stream.
    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Returns a mutable reference to the TCP stream.
    pub fn stream_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }

    /// Returns the remote backend address.
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// Returns the time when the connection was established.
    pub fn connected_at(&self) -> Instant {
        self.connected_at
    }
}
