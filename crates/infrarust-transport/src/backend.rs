//! Backend connection management.
//!
//! `BackendConnector` handles connecting to backend Minecraft servers
//! with failover, timeout, and optional proxy protocol v2.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use socket2::Socket;
use tokio::net::TcpStream;
use tokio::time::Instant;

use infrarust_config::{KeepaliveConfig, ServerAddress};

use crate::connection::ConnectionInfo;
use crate::error::TransportError;
use crate::proxy_protocol::encode_proxy_protocol_v2;
use crate::socket::configure_stream_socket;

pub struct ConnectAttempt<'a> {
    pub server_id: &'a str,
    pub address: &'a ServerAddress,
    pub elapsed: Duration,
    pub error: Option<&'a TransportError>,
}

impl ConnectAttempt<'_> {
    pub const fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

pub trait ConnectAttemptObserver: Send + Sync {
    fn on_attempt(&self, attempt: &ConnectAttempt<'_>);
}

/// Connects to backend servers with failover and timeout.
#[derive(Clone)]
pub struct BackendConnector {
    /// Default connection timeout.
    pub default_timeout: Duration,
    /// TCP keepalive configuration for backend connections.
    pub keepalive: KeepaliveConfig,
    /// Addresses tried before giving up. `0` means no limit.
    max_attempts: usize,
    /// Notified of every connection attempt outcome (per address).
    observer: Option<Arc<dyn ConnectAttemptObserver>>,
}

impl std::fmt::Debug for BackendConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendConnector")
            .field("default_timeout", &self.default_timeout)
            .field("keepalive", &self.keepalive)
            .field("max_attempts", &self.max_attempts)
            .field("has_observer", &self.observer.is_some())
            .finish()
    }
}

impl BackendConnector {
    pub const fn new(default_timeout: Duration, keepalive: KeepaliveConfig) -> Self {
        Self {
            default_timeout,
            keepalive,
            max_attempts: 0,
            observer: None,
        }
    }

    #[must_use]
    pub const fn with_max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn ConnectAttemptObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Connects to one of the given backend addresses.
    ///
    /// Tries each address in order. On the first successful connection,
    /// configures the socket (`TCP_NODELAY`, keepalive) and optionally
    /// sends a proxy protocol v2 header.
    ///
    /// Returns `AllBackendsFailed` if no address could be reached.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::AllBackendsFailed`] if none of the provided
    /// addresses could be reached, or a connection-specific error from the
    /// last failed attempt.
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

        let attempts = if self.max_attempts == 0 {
            addresses.len()
        } else {
            self.max_attempts.min(addresses.len())
        };

        for address in &addresses[..attempts] {
            let started = Instant::now();
            let outcome = self
                .try_connect(address, timeout, send_proxy_protocol, client_info)
                .await;
            let elapsed = started.elapsed();

            match outcome {
                Ok(conn) => {
                    if let Some(ref observer) = self.observer {
                        observer.on_attempt(&ConnectAttempt {
                            server_id,
                            address,
                            elapsed,
                            error: None,
                        });
                    }
                    return Ok(conn);
                }
                Err(e) => {
                    tracing::warn!(
                        server_id = server_id,
                        address = %address,
                        error = %e,
                        "backend connection attempt failed"
                    );
                    if let Some(ref observer) = self.observer {
                        observer.on_attempt(&ConnectAttempt {
                            server_id,
                            address,
                            elapsed,
                            error: Some(&e),
                        });
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(
            last_error.unwrap_or_else(|| TransportError::AllBackendsFailed {
                server_id: server_id.to_string(),
            }),
        )
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
            .map_err(|_| TransportError::ConnectTimeout {
                addr: connect_addr.clone(),
                timeout,
            })?
            .map_err(|source| TransportError::BackendConnect {
                addr: connect_addr,
                source,
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
            server_address: address.clone(),
            connected_at: Instant::now(),
        })
    }
}

/// An established connection to a backend server.
#[derive(Debug)]
pub struct BackendConnection {
    stream: TcpStream,
    remote_addr: SocketAddr,
    server_address: ServerAddress,
    connected_at: Instant,
}

impl BackendConnection {
    /// Consumes the connection and returns the TCP stream.
    pub fn into_stream(self) -> TcpStream {
        self.stream
    }

    /// Returns a reference to the TCP stream.
    pub const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Returns a mutable reference to the TCP stream.
    pub const fn stream_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }

    /// Returns the remote backend address.
    pub const fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    pub const fn server_address(&self) -> &ServerAddress {
        &self.server_address
    }

    /// Returns the time when the connection was established.
    pub const fn connected_at(&self) -> Instant {
        self.connected_at
    }
}
