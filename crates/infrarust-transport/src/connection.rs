//! Client connection types.
//!
//! `ClientConnection` wraps a TCP stream with metadata about the
//! connected client. `ConnectionInfo` is a lightweight clone of
//! the metadata without the stream.

use std::net::{IpAddr, SocketAddr};

use bytes::BytesMut;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::error::TransportError;
use crate::proxy_protocol::ProxyProtocolInfo;

/// An accepted client connection with metadata.
#[derive(Debug)]
pub struct ClientConnection {
    stream: TcpStream,
    peer_addr: SocketAddr,
    real_ip: Option<IpAddr>,
    real_port: Option<u16>,
    local_addr: SocketAddr,
    connected_at: Instant,
    buffered_data: BytesMut,
}

impl ClientConnection {
    pub fn new(stream: TcpStream, peer_addr: SocketAddr, local_addr: SocketAddr) -> Self {
        Self {
            stream,
            peer_addr,
            real_ip: None,
            real_port: None,
            local_addr,
            connected_at: Instant::now(),
            buffered_data: BytesMut::new(),
        }
    }

    /// Sets the real client address from proxy protocol info.
    #[must_use]
    pub const fn with_proxy_protocol(mut self, info: &ProxyProtocolInfo) -> Self {
        self.real_ip = Some(info.source_addr.ip());
        self.real_port = Some(info.source_addr.port());
        self
    }

    /// This is used when the proxy protocol decode over-reads (TCP coalescing
    /// causes the first Minecraft packet to be read together with the PP header).
    /// The leftover bytes are injected here so they are consumed before reading
    /// from the stream.
    pub fn inject_buffered_data(&mut self, data: &[u8]) {
        self.buffered_data.extend_from_slice(data);
    }

    /// Returns the effective client IP address.
    ///
    /// If proxy protocol provided a real IP, returns that.
    /// Otherwise returns the TCP peer address.
    pub fn client_addr(&self) -> IpAddr {
        self.real_ip.unwrap_or_else(|| self.peer_addr.ip())
    }

    /// Consumes the connection, returning the stream, buffered data, and metadata.
    pub fn into_parts(self) -> (TcpStream, BytesMut, ConnectionInfo) {
        let info = ConnectionInfo {
            peer_addr: self.peer_addr,
            real_ip: self.real_ip,
            real_port: self.real_port,
            local_addr: self.local_addr,
            connected_at: self.connected_at,
        };
        (self.stream, self.buffered_data, info)
    }

    /// Reads up to `n` bytes from the stream into the internal buffer
    /// without consuming them from the connection.
    ///
    /// Returns a slice of the buffered data.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Forward`] if reading from the stream fails.
    pub async fn peek(&mut self, n: usize) -> Result<&[u8], TransportError> {
        if self.buffered_data.len() < n {
            let needed = n - self.buffered_data.len();
            self.buffered_data.reserve(needed);
            let mut buf = vec![0u8; needed];
            let read = self
                .stream
                .read(&mut buf[..needed])
                .await
                .map_err(TransportError::Forward)?;
            self.buffered_data.extend_from_slice(&buf[..read]);
        }
        Ok(&self.buffered_data[..self.buffered_data.len().min(n)])
    }

    /// Returns the TCP peer address.
    pub const fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Returns the real client IP from proxy protocol, if available.
    pub const fn real_ip(&self) -> Option<IpAddr> {
        self.real_ip
    }

    /// Returns the local listener address.
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns the time when the connection was accepted.
    pub const fn connected_at(&self) -> Instant {
        self.connected_at
    }

    /// Returns a reference to the underlying TCP stream.
    pub const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Returns a mutable reference to the underlying TCP stream.
    pub const fn stream_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }
}

/// Lightweight metadata about a client connection (without the stream).
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    /// TCP peer address (may be a load balancer).
    pub peer_addr: SocketAddr,
    /// Real client IP from proxy protocol.
    pub real_ip: Option<IpAddr>,
    /// Real client port from proxy protocol.
    pub real_port: Option<u16>,
    /// Local listener address.
    pub local_addr: SocketAddr,
    /// Time when the connection was accepted.
    pub connected_at: Instant,
}
