//! TCP listener with semaphore-bounded accept loop.
//!
//! `Listener` wraps a `tokio::net::TcpListener` with connection limits,
//! graceful shutdown, optional proxy protocol decoding, and socket
//! configuration on accepted connections.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use ipnet::IpNet;
use socket2::Socket;
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use infrarust_config::KeepaliveConfig;

use crate::connection::{ClientConnection, canonicalize_ip};
use crate::error::TransportError;
use crate::proxy_protocol::decode_proxy_protocol;
use crate::socket::{configure_listener_socket, configure_stream_socket, into_tokio_listener};

/// Configuration for the listener.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Address to bind to.
    pub bind: SocketAddr,
    /// Maximum concurrent connections (0 = unlimited).
    pub max_connections: u32,
    /// TCP keepalive configuration for accepted connections.
    pub keepalive: KeepaliveConfig,
    /// Enable `SO_REUSEPORT` (Linux only).
    pub so_reuseport: bool,
    /// Decode proxy protocol headers on accepted connections.
    pub receive_proxy_protocol: bool,
    /// Peers allowed to send a proxy protocol header.
    pub trusted_proxies: Vec<IpNet>,
}

/// TCP listener with connection limiting and graceful shutdown.
#[derive(Debug)]
pub struct Listener {
    inner: TcpListener,
    semaphore: Option<Arc<Semaphore>>,
    config: ListenerConfig,
    shutdown: CancellationToken,
}

#[derive(Debug, Clone, Copy)]
enum ProxyHeader {
    NotExpected,
    Expected,
    UntrustedPeer,
}

impl ProxyHeader {
    fn for_peer(config: &ListenerConfig, peer: IpAddr) -> Self {
        if !config.receive_proxy_protocol {
            return Self::NotExpected;
        }
        let peer = canonicalize_ip(peer);
        if config.trusted_proxies.iter().any(|net| net.contains(&peer)) {
            Self::Expected
        } else {
            Self::UntrustedPeer
        }
    }
}

#[derive(Debug)]
pub struct PendingConnection {
    connection: ClientConnection,
    proxy_header: ProxyHeader,
    permit: Option<OwnedSemaphorePermit>,
}

impl PendingConnection {
    pub const fn peer_addr(&self) -> SocketAddr {
        self.connection.peer_addr()
    }

    pub async fn resolve(self) -> Result<AcceptedConnection, TransportError> {
        let Self {
            mut connection,
            proxy_header,
            permit,
        } = self;
        match proxy_header {
            ProxyHeader::NotExpected => {}
            ProxyHeader::UntrustedPeer => {
                return Err(TransportError::UntrustedProxyPeer {
                    peer: canonicalize_ip(connection.peer_addr().ip()),
                });
            }
            ProxyHeader::Expected => {
                let (info, leftover) = decode_proxy_protocol(connection.stream_mut()).await?;
                if let Some(info) = info {
                    connection = connection.with_proxy_protocol(&info);
                }
                connection.inject_buffered_data(&leftover);
            }
        }
        Ok(AcceptedConnection { connection, permit })
    }
}

/// An accepted connection with its semaphore permit.
///
/// The permit is held for the lifetime of the connection.
/// When dropped, the permit is released, allowing new connections.
pub struct AcceptedConnection {
    /// The accepted client connection.
    pub connection: ClientConnection,
    /// Semaphore permit (released on drop).
    permit: Option<OwnedSemaphorePermit>,
}

impl AcceptedConnection {
    pub fn into_parts(self) -> (ClientConnection, Option<OwnedSemaphorePermit>) {
        (self.connection, self.permit)
    }
}

impl std::fmt::Debug for AcceptedConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptedConnection")
            .field("connection", &self.connection)
            .field("haspermit", &self.permit.is_some())
            .finish()
    }
}

impl Listener {
    /// Binds to the configured address and prepares the listener.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Bind`] if the socket cannot be bound, or
    /// [`TransportError::SocketConfig`] if socket configuration fails.
    #[allow(clippy::unused_async)] // Kept async for API consistency; callers already await this.
    pub async fn bind(
        config: ListenerConfig,
        shutdown: CancellationToken,
    ) -> Result<Self, TransportError> {
        let socket = configure_listener_socket(config.bind, config.so_reuseport)?;
        let inner = into_tokio_listener(socket)?;
        let local_addr = inner.local_addr().map_err(TransportError::SocketConfig)?;

        let semaphore = if config.max_connections > 0 {
            Some(Arc::new(Semaphore::new(config.max_connections as usize)))
        } else {
            None
        };

        tracing::info!(bind = %local_addr, max_connections = config.max_connections, "listener bound");

        Ok(Self {
            inner,
            semaphore,
            config,
            shutdown,
        })
    }

    /// Accepts the next connection.
    ///
    /// Acquires a semaphore permit (if connection limiting is enabled),
    /// waits for an incoming connection, and configures the socket.
    ///
    /// Transient errors (EMFILE, ENOMEM) are retried with backoff.
    /// Returns `TransportError::Shutdown` when the shutdown token is cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Shutdown`] if the shutdown token is
    /// cancelled, [`TransportError::Accept`] on a non-transient accept
    /// error, or [`TransportError::SocketConfig`] if socket configuration
    /// fails.
    pub async fn accept(&self) -> Result<PendingConnection, TransportError> {
        let mut backoff = std::time::Duration::from_millis(100);
        loop {
            // Acquire permit before accepting
            let permit = if let Some(sem) = &self.semaphore {
                let permit = tokio::select! {
                    biased;
                    () = self.shutdown.cancelled() => {
                        return Err(TransportError::Shutdown);
                    }
                    result = sem.clone().acquire_owned() => {
                        result.map_err(|_| TransportError::Shutdown)?
                    }
                };
                Some(permit)
            } else {
                None
            };

            // Accept connection
            let (stream, peer_addr) = tokio::select! {
                biased;
                () = self.shutdown.cancelled() => {
                    return Err(TransportError::Shutdown);
                }
                result = self.inner.accept() => {
                    match result {
                        Ok((stream, addr)) => (stream, addr),
                        Err(e) if is_transient_error(&e) => {
                            tracing::warn!(error = %e, backoff_ms = backoff.as_millis(), "transient accept error, retrying");
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
                            continue;
                        }
                        Err(e) => return Err(TransportError::Accept(e)),
                    }
                }
            };

            // Configure accepted socket via socket2 round-trip
            let std_stream = stream.into_std().map_err(TransportError::SocketConfig)?;
            let socket = Socket::from(std_stream);
            configure_stream_socket(&socket, &self.config.keepalive)?;
            socket
                .set_nonblocking(true)
                .map_err(TransportError::SocketConfig)?;
            let std_stream: std::net::TcpStream = socket.into();
            let stream = tokio::net::TcpStream::from_std(std_stream)
                .map_err(TransportError::SocketConfig)?;

            let local_addr = self
                .inner
                .local_addr()
                .map_err(TransportError::SocketConfig)?;

            return Ok(PendingConnection {
                connection: ClientConnection::new(stream, peer_addr, local_addr),
                proxy_header: ProxyHeader::for_peer(&self.config, peer_addr.ip()),
                permit,
            });
        }
    }

    /// Returns the local address the listener is bound to.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::SocketConfig`] if the address cannot be
    /// retrieved from the underlying socket.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.inner
            .local_addr()
            .map_err(TransportError::SocketConfig)
    }
}

/// Checks if an accept error is transient and should be retried.
fn is_transient_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset
    ) || matches!(e.raw_os_error(), Some(24 | 23 | 12 | 105))
    // 24=EMFILE, 23=ENFILE, 12=ENOMEM, 105=ENOBUFS (Linux values)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn proxy_config(trusted: &[&str]) -> ListenerConfig {
        ListenerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_connections: 0,
            keepalive: KeepaliveConfig::default(),
            so_reuseport: false,
            receive_proxy_protocol: true,
            trusted_proxies: trusted.iter().map(|net| net.parse().unwrap()).collect(),
        }
    }

    #[test]
    fn an_ipv4_mapped_peer_matches_ipv4_trusted_networks() {
        let config = proxy_config(&["10.0.0.0/8"]);
        let mapped: IpAddr = "::ffff:10.1.2.3".parse().unwrap();
        assert!(matches!(
            ProxyHeader::for_peer(&config, mapped),
            ProxyHeader::Expected
        ));
    }

    #[test]
    fn an_ipv4_mapped_peer_does_not_match_ipv6_networks() {
        let config = proxy_config(&["::/0"]);
        let mapped: IpAddr = "::ffff:10.1.2.3".parse().unwrap();
        assert!(matches!(
            ProxyHeader::for_peer(&config, mapped),
            ProxyHeader::UntrustedPeer
        ));
    }

    #[test]
    fn an_empty_trust_list_refuses_every_peer() {
        let config = proxy_config(&[]);
        assert!(matches!(
            ProxyHeader::for_peer(&config, "127.0.0.1".parse().unwrap()),
            ProxyHeader::UntrustedPeer
        ));
    }

    #[test]
    fn transient_errors_are_retried() {
        // EMFILE, ENFILE, ENOMEM, ENOBUFS (Linux values)
        for errno in [24, 23, 12, 105] {
            assert!(
                is_transient_error(&std::io::Error::from_raw_os_error(errno)),
                "errno {errno} should be transient"
            );
        }
        for kind in [
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::ConnectionReset,
        ] {
            assert!(is_transient_error(&std::io::Error::from(kind)));
        }
    }

    #[test]
    fn fatal_errors_are_not_retried() {
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidInput,
        ] {
            assert!(!is_transient_error(&std::io::Error::from(kind)));
        }
    }
}
