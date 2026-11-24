use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::TcpStream;
use tokio::time::Instant;

use infrarust_transport::{AcceptedConnection, ClientConnection, ConnectionInfo, ConnectionSlot};

use crate::error::CoreError;
use crate::filter::transport_chain::TransportSession;
use crate::hot_restart::InFlight;
use crate::loadbalancer::PendingTicket;
use crate::pipeline::types::{BackendTargets, HandshakeData, LegacyHello, LoginData, RoutingData};
use infrarust_protocol::ProtocolError;

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

fn next_connection_id() -> u64 {
    NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn connection_id_watermark() -> u64 {
    NEXT_CONNECTION_ID.load(Ordering::Relaxed)
}

pub(crate) fn reserve_connection_ids(next: u64) {
    NEXT_CONNECTION_ID.fetch_max(next, Ordering::Relaxed);
}

pub struct ConnectionContext {
    stream: Option<TcpStream>,
    info: ConnectionInfo,
    pub buffered_data: BytesMut,
    pub handshake: Option<HandshakeData>,
    pub routing: Option<RoutingData>,
    pub login: Option<LoginData>,
    pub legacy_hello: Option<LegacyHello>,
    pub layout_refusal: Option<ProtocolError>,
    pub backend_targets: Option<BackendTargets>,
    pub pending_ticket: Option<PendingTicket>,
    pub span: Option<tracing::Span>,
    pub connection_id: u64,
    transport: Option<TransportSession>,
    in_flight: Option<InFlight>,
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
            handshake: None,
            routing: None,
            login: None,
            legacy_hello: None,
            layout_refusal: None,
            backend_targets: None,
            pending_ticket: None,
            span: None,
            connection_id: next_connection_id(),
            transport: None,
            in_flight: None,
            _slot: slot,
        }
    }

    pub fn require_handshake(&self) -> Result<&HandshakeData, CoreError> {
        self.handshake
            .as_ref()
            .ok_or(CoreError::MissingExtension("HandshakeData"))
    }

    pub fn require_routing(&self) -> Result<&RoutingData, CoreError> {
        self.routing
            .as_ref()
            .ok_or(CoreError::MissingExtension("RoutingData"))
    }

    pub fn require_login(&self) -> Result<&LoginData, CoreError> {
        self.login
            .as_ref()
            .ok_or(CoreError::MissingExtension("LoginData"))
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

    pub fn detach_transport(&mut self) {
        if let Some(mut session) = self.transport.take() {
            session.detach();
        }
    }

    pub fn hold_in_flight(&mut self, guard: InFlight) {
        self.in_flight = Some(guard);
    }

    pub fn settle(&mut self) {
        self.in_flight = None;
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

    #[test]
    fn reserved_ids_are_never_handed_out_again() {
        let floor = connection_id_watermark() + 1_000;
        reserve_connection_ids(floor);
        reserve_connection_ids(floor - 500);
        assert!(connection_id_watermark() >= floor);
        assert!(next_connection_id() >= floor);
    }

    #[tokio::test]
    async fn settling_releases_the_in_flight_guard() {
        let registry = std::sync::Arc::new(crate::hot_restart::RelayRegistry::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let _client = TcpStream::connect(local).await.unwrap();
        let (stream, peer) = listener.accept().await.unwrap();
        let mut ctx = ConnectionContext::new_for_test(stream, peer, local);

        ctx.hold_in_flight(registry.in_flight_guard());
        assert_eq!(registry.in_flight(), 1);
        ctx.settle();
        assert_eq!(registry.in_flight(), 0);

        ctx.hold_in_flight(registry.in_flight_guard());
        drop(ctx);
        assert_eq!(registry.in_flight(), 0);
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
}
