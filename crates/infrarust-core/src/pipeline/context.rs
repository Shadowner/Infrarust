use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::TcpStream;
use tokio::time::Instant;

use infrarust_transport::{AcceptedConnection, ClientConnection, ConnectionInfo, ConnectionSlot};

use crate::error::CoreError;
use crate::filter::transport_chain::TransportSession;

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

fn next_connection_id() -> u64 {
    NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Default)]
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(val))
            .and_then(|prev| prev.downcast().ok().map(|b| *b))
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
    }

    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut())
    }

    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast().ok().map(|b| *b))
    }
}

pub struct ConnectionContext {
    stream: Option<TcpStream>,
    info: ConnectionInfo,
    pub buffered_data: BytesMut,
    pub extensions: Extensions,
    pub connection_id: u64,
    transport: Option<TransportSession>,
    _slot: ConnectionSlot,
}

impl ConnectionContext {
    pub fn from_accepted(accepted: AcceptedConnection) -> Self {
        let (connection, slot) = accepted.into_parts();
        Self::from_parts(connection, slot)
    }

    fn from_parts(connection: ClientConnection, slot: ConnectionSlot) -> Self {
        let (stream, buffered_data, info) = connection.into_parts();

        Self {
            stream: Some(stream),
            info,
            buffered_data,
            extensions: Extensions::new(),
            connection_id: next_connection_id(),
            transport: None,
            _slot: slot,
        }
    }

    pub fn require_extension<T: Send + Sync + 'static>(
        &self,
        name: &'static str,
    ) -> Result<&T, CoreError> {
        self.extensions
            .get::<T>()
            .ok_or(CoreError::MissingExtension(name))
    }

    #[allow(clippy::expect_used)]
    pub const fn stream(&self) -> &TcpStream {
        self.stream.as_ref().expect("stream already taken")
    }

    #[allow(clippy::expect_used)]
    pub const fn stream_mut(&mut self) -> &mut TcpStream {
        self.stream.as_mut().expect("stream already taken")
    }

    #[allow(clippy::expect_used)]
    pub const fn take_stream(&mut self) -> TcpStream {
        self.stream.take().expect("stream already taken")
    }

    pub const fn has_stream(&self) -> bool {
        self.stream.is_some()
    }

    pub fn new_for_test(stream: TcpStream, peer_addr: SocketAddr, local_addr: SocketAddr) -> Self {
        Self::from_parts(
            ClientConnection::new(stream, peer_addr, local_addr),
            ConnectionSlot::default(),
        )
    }

    pub fn attach_transport(&mut self, session: TransportSession) {
        self.transport = Some(session);
    }

    pub fn client_addr(&self) -> SocketAddr {
        self.info.client_addr()
    }

    pub fn client_ip(&self) -> IpAddr {
        self.client_addr().ip()
    }

    pub fn real_ip(&self) -> Option<IpAddr> {
        self.info.real_addr.map(|_| self.client_ip())
    }

    pub const fn peer_addr(&self) -> SocketAddr {
        self.info.peer_addr
    }

    pub const fn local_addr(&self) -> SocketAddr {
        self.info.local_addr
    }

    pub const fn connected_at(&self) -> Instant {
        self.info.connected_at
    }

    pub fn connection_duration(&self) -> Duration {
        self.info.connected_at.elapsed()
    }

    pub const fn connection_info(&self) -> &ConnectionInfo {
        &self.info
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use infrarust_transport::{ProxyProtocolInfo, ProxyProtocolVersion};

    use super::*;

    #[tokio::test]
    async fn the_proxy_protocol_source_port_reaches_the_backend_connection_info() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let _client = TcpStream::connect(local).await.unwrap();
        let (stream, peer) = listener.accept().await.unwrap();
        let source: SocketAddr = "[::ffff:203.0.113.7]:40123".parse().unwrap();
        let header = ProxyProtocolInfo {
            source_addr: source,
            dest_addr: local,
            version: ProxyProtocolVersion::V2,
        };

        let ctx = ConnectionContext::from_parts(
            ClientConnection::new(stream, peer, local).with_proxy_protocol(&header),
            ConnectionSlot::default(),
        );

        let client: SocketAddr = "203.0.113.7:40123".parse().unwrap();
        assert_eq!(ctx.client_addr(), client);
        assert_eq!(ctx.connection_info().client_addr(), client);
        assert_eq!(ctx.real_ip(), Some(client.ip()));
        assert_eq!(ctx.peer_addr(), peer);
    }

    #[tokio::test]
    async fn a_direct_connection_has_no_real_ip() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let _client = TcpStream::connect(local).await.unwrap();
        let (stream, peer) = listener.accept().await.unwrap();

        let ctx = ConnectionContext::new_for_test(stream, peer, local);

        assert_eq!(ctx.client_addr(), peer);
        assert_eq!(ctx.real_ip(), None);
    }

    #[test]
    fn extensions_insert_and_get() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        assert_eq!(ext.get::<u32>(), Some(&42));
    }

    #[test]
    fn extensions_get_missing_returns_none() {
        let ext = Extensions::new();
        assert_eq!(ext.get::<u32>(), None);
    }

    #[test]
    fn extensions_overwrite_returns_previous() {
        let mut ext = Extensions::new();
        ext.insert(1u32);
        let prev = ext.insert(2u32);
        assert_eq!(prev, Some(1));
        assert_eq!(ext.get::<u32>(), Some(&2));
    }

    #[test]
    fn extensions_different_types_coexist() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        ext.insert("hello".to_string());
        assert_eq!(ext.get::<u32>(), Some(&42));
        assert_eq!(ext.get::<String>(), Some(&"hello".to_string()));
    }

    #[test]
    fn extensions_contains() {
        let mut ext = Extensions::new();
        assert!(!ext.contains::<u32>());
        ext.insert(1u32);
        assert!(ext.contains::<u32>());
    }

    #[test]
    fn extensions_remove() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        let removed = ext.remove::<u32>();
        assert_eq!(removed, Some(42));
        assert!(!ext.contains::<u32>());
    }

    #[test]
    fn extensions_get_mut() {
        let mut ext = Extensions::new();
        ext.insert(vec![1, 2, 3]);
        ext.get_mut::<Vec<i32>>().unwrap().push(4);
        assert_eq!(ext.get::<Vec<i32>>(), Some(&vec![1, 2, 3, 4]));
    }
}
