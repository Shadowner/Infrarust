use std::net::{IpAddr, SocketAddr};

use bytes::{BufMut, BytesMut};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::error::TransportError;
use crate::proxy_protocol::ProxyProtocolInfo;

#[derive(Debug)]
pub struct ClientConnection {
    stream: TcpStream,
    info: ConnectionInfo,
    buffered_data: BytesMut,
}

impl ClientConnection {
    pub fn new(stream: TcpStream, peer_addr: SocketAddr, local_addr: SocketAddr) -> Self {
        Self {
            stream,
            info: ConnectionInfo {
                peer_addr,
                real_addr: None,
                local_addr,
                connected_at: Instant::now(),
            },
            buffered_data: BytesMut::new(),
        }
    }

    #[must_use]
    pub const fn with_proxy_protocol(mut self, info: &ProxyProtocolInfo) -> Self {
        self.info.real_addr = Some(info.source_addr);
        self
    }

    pub fn inject_buffered_data(&mut self, data: &[u8]) {
        self.buffered_data.extend_from_slice(data);
    }

    pub fn client_addr(&self) -> IpAddr {
        self.info.client_addr().ip()
    }

    pub fn into_parts(self) -> (TcpStream, BytesMut, ConnectionInfo) {
        (self.stream, self.buffered_data, self.info)
    }

    pub async fn peek(&mut self, n: usize) -> Result<&[u8], TransportError> {
        if self.buffered_data.len() < n {
            let needed = n - self.buffered_data.len();
            self.buffered_data.reserve(needed);
            self.stream
                .read_buf(&mut (&mut self.buffered_data).limit(needed))
                .await
                .map_err(TransportError::Io)?;
        }
        Ok(&self.buffered_data[..self.buffered_data.len().min(n)])
    }

    pub const fn peer_addr(&self) -> SocketAddr {
        self.info.peer_addr
    }

    pub fn real_ip(&self) -> Option<IpAddr> {
        self.info.real_addr.map(|addr| addr.ip())
    }

    pub const fn local_addr(&self) -> SocketAddr {
        self.info.local_addr
    }

    pub const fn connected_at(&self) -> Instant {
        self.info.connected_at
    }

    pub const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    pub const fn stream_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }
}

pub(crate) fn canonicalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub peer_addr: SocketAddr,
    pub real_addr: Option<SocketAddr>,
    pub local_addr: SocketAddr,
    pub connected_at: Instant,
}

impl ConnectionInfo {
    pub fn client_addr(&self) -> SocketAddr {
        let addr = self.real_addr.unwrap_or(self.peer_addr);
        SocketAddr::new(canonicalize_ip(addr.ip()), addr.port())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn canonicalize_maps_ipv4_mapped_v6_to_v4() {
        let mapped = IpAddr::V6(Ipv4Addr::new(1, 2, 3, 4).to_ipv6_mapped());
        assert_eq!(
            canonicalize_ip(mapped),
            IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))
        );
    }

    #[test]
    fn canonicalize_leaves_native_v6_untouched() {
        let v6 = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1);
        assert_eq!(canonicalize_ip(IpAddr::V6(v6)), IpAddr::V6(v6));
    }

    #[test]
    fn canonicalize_leaves_v4_untouched() {
        let v4 = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));
        assert_eq!(canonicalize_ip(v4), v4);
    }
}
