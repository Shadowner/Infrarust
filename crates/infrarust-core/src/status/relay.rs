//! Status relay client.
//!
//! Connects to a backend Minecraft server, performs the full status handshake
//! (Handshake → `StatusRequest` → `StatusResponse` → Ping → Pong), and returns
//! the parsed response with measured latency.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use infrarust_config::{DomainRewrite, ServerConfig};
use infrarust_protocol::Packet;
use infrarust_protocol::codec::VarInt;
use infrarust_protocol::io::{PacketDecoder, PacketEncoder};
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::status::{
    CPingResponse, CStatusResponse, SPingRequest, SStatusRequest,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_transport::BackendConnector;
use infrarust_transport::connection::ConnectionInfo;

use super::STATUS_PROTOCOL_VERSION;
use super::response::ServerPingResponse;
use crate::error::CoreError;

/// Result of a successful status relay.
#[derive(Debug)]
pub struct RelayResult {
    /// The parsed status response from the backend.
    pub response: ServerPingResponse,
    /// Round-trip latency of the ping/pong exchange.
    pub latency: Duration,
}

/// Lightweight client for relaying status pings to backends.
pub struct StatusRelayClient {
    backend_connector: Arc<BackendConnector>,
    registry: Arc<PacketRegistry>,
    timeout: Duration,
}

impl StatusRelayClient {
    pub const fn new(
        backend_connector: Arc<BackendConnector>,
        registry: Arc<PacketRegistry>,
        timeout: Duration,
    ) -> Self {
        Self {
            backend_connector,
            registry,
            timeout,
        }
    }

    /// Relays a status ping to the backend and returns the parsed response.
    ///
    /// # Errors
    /// Returns `CoreError` on connection failure, protocol error, or timeout.
    pub async fn relay(
        &self,
        server_id: &str,
        server_config: &ServerConfig,
        handshake_domain: &str,
        protocol_version: ProtocolVersion,
        client_info: &ConnectionInfo,
    ) -> Result<RelayResult, CoreError> {
        tokio::time::timeout(
            self.timeout,
            self.relay_inner(
                server_id,
                server_config,
                handshake_domain,
                protocol_version,
                client_info,
            ),
        )
        .await
        .map_err(|_| {
            CoreError::Other(format!(
                "status relay timeout after {:?} for '{}'",
                self.timeout, server_id
            ))
        })?
    }

    async fn relay_inner(
        &self,
        server_id: &str,
        server_config: &ServerConfig,
        handshake_domain: &str,
        protocol_version: ProtocolVersion,
        client_info: &ConnectionInfo,
    ) -> Result<RelayResult, CoreError> {
        let addresses = &server_config.addresses;
        let send_proxy_protocol = server_config.send_proxy_protocol;

        // 1. Connect to backend
        let conn = self
            .backend_connector
            .connect(server_id, addresses, None, send_proxy_protocol, client_info)
            .await?;
        let mut stream = conn.into_stream();

        // 2. Resolve domain for the handshake
        let relay_domain = resolve_relay_domain(handshake_domain, server_config);
        let backend_port = addresses.first().map_or(25565, |a| a.port);

        // 3. Send SHandshake (intent = Status)
        let handshake = SHandshake {
            protocol_version: VarInt(protocol_version.0),
            server_address: relay_domain,
            server_port: backend_port,
            next_state: ConnectionState::Status,
        };
        send_packet(&self.registry, &mut stream, &handshake, protocol_version).await?;

        // 4. Send SStatusRequest (empty)
        send_packet(
            &self.registry,
            &mut stream,
            &SStatusRequest,
            STATUS_PROTOCOL_VERSION,
        )
        .await?;

        // Use a single persistent decoder for the entire exchange to avoid
        // losing data when TCP delivers multiple packets in one read.
        let mut decoder = PacketDecoder::new();

        // 5. Read CStatusResponse
        let status_frame = read_next_frame(&mut stream, &mut decoder).await?;
        let status_decoded = self.registry.decode_frame(
            &status_frame,
            ConnectionState::Status,
            Direction::Clientbound,
            STATUS_PROTOCOL_VERSION,
        )?;

        let json_response = match status_decoded {
            DecodedPacket::Typed { packet, .. } => packet
                .as_any()
                .downcast_ref::<CStatusResponse>()
                .map(|p| p.json_response.clone())
                .ok_or_else(|| {
                    CoreError::Other("unexpected packet type for status response".into())
                })?,
            DecodedPacket::Opaque { .. } => {
                return Err(CoreError::Other("received opaque status response".into()));
            }
        };

        // 6. Parse JSON
        let response: ServerPingResponse = serde_json::from_str(&json_response).map_err(|e| {
            CoreError::Other(format!("invalid status JSON from '{server_id}': {e}"))
        })?;

        // 7. Ping/Pong for latency measurement
        let ping_payload = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let ping_start = tokio::time::Instant::now();
        send_packet(
            &self.registry,
            &mut stream,
            &SPingRequest {
                payload: ping_payload,
            },
            STATUS_PROTOCOL_VERSION,
        )
        .await?;

        // 8. Read CPingResponse
        let pong_frame = read_next_frame(&mut stream, &mut decoder).await?;
        let latency = ping_start.elapsed();

        let pong_decoded = self.registry.decode_frame(
            &pong_frame,
            ConnectionState::Status,
            Direction::Clientbound,
            STATUS_PROTOCOL_VERSION,
        )?;

        if let DecodedPacket::Typed { packet, .. } = pong_decoded
            && let Some(pong) = packet.as_any().downcast_ref::<CPingResponse>()
            && pong.payload != ping_payload
        {
            tracing::debug!(
                expected = ping_payload,
                got = pong.payload,
                "ping/pong payload mismatch (non-fatal)"
            );
        }

        // 9. Connection is dropped (closed) here
        Ok(RelayResult { response, latency })
    }
}

/// Applies domain rewrite for the relay handshake.
fn resolve_relay_domain(handshake_domain: &str, server_config: &ServerConfig) -> String {
    match &server_config.domain_rewrite {
        DomainRewrite::Explicit(domain) => domain.clone(),
        DomainRewrite::FromBackend => server_config
            .addresses
            .first()
            .map_or_else(|| handshake_domain.to_string(), |a| a.host.clone()),
        _ => handshake_domain.to_string(),
    }
}

/// Encodes and sends a typed packet on the stream.
async fn send_packet<P: Packet>(
    registry: &PacketRegistry,
    stream: &mut tokio::net::TcpStream,
    packet: &P,
    version: ProtocolVersion,
) -> Result<(), CoreError> {
    let packet_id = registry
        .get_packet_id::<P>(P::state(), P::direction(), version)
        .unwrap_or(0);

    let mut payload = Vec::new();
    packet.encode(&mut payload, version)?;

    let mut encoder = PacketEncoder::new();
    encoder.append_raw(packet_id, &payload)?;
    let bytes = encoder.take();

    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// Reads the next packet frame from the stream using a persistent decoder.
///
/// The decoder must be reused across calls on the same stream to avoid
/// losing data when TCP delivers multiple packets in a single read.
async fn read_next_frame(
    stream: &mut tokio::net::TcpStream,
    decoder: &mut PacketDecoder,
) -> Result<infrarust_protocol::io::PacketFrame, CoreError> {
    let mut buf = [0u8; 4096];

    loop {
        if let Some(frame) = decoder.try_next_frame()? {
            return Ok(frame);
        }
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(CoreError::ConnectionClosed);
        }
        decoder.queue_bytes(&buf[..n]);
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::items_after_statements,
        clippy::default_trait_access,
        clippy::similar_names
    )]
    use super::*;
    use infrarust_config::{KeepaliveConfig, ServerAddress};
    use infrarust_protocol::build_default_registry;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    const TEST_JSON: &str = r#"{"version":{"name":"1.21.4","protocol":769},"players":{"max":100,"online":42},"description":{"text":"Test Server"}}"#;

    /// Spawns a mock MC server that handles a single status exchange.
    async fn spawn_mock_mc_status(response_json: &str) -> (Vec<ServerAddress>, CancellationToken) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = CancellationToken::new();
        let json = response_json.to_string();
        let s = shutdown.clone();
        let registry = Arc::new(build_default_registry());

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (mut stream, _) = result.unwrap();
                        let registry = Arc::clone(&registry);
                        let json = json.clone();

                        tokio::spawn(async move {
                            let version = ProtocolVersion::V1_21;
                            let mut decoder = PacketDecoder::new();

                            // Read SHandshake
                            let _ = read_next_frame(&mut stream, &mut decoder).await.unwrap();

                            // Read SStatusRequest
                            let _ = read_next_frame(&mut stream, &mut decoder).await.unwrap();

                            // Send CStatusResponse
                            let resp = CStatusResponse { json_response: json };
                            send_packet(&registry, &mut stream, &resp, version).await.unwrap();

                            // Read SPingRequest
                            let ping_frame = read_next_frame(&mut stream, &mut decoder).await.unwrap();
                            let decoded = registry.decode_frame(
                                &ping_frame,
                                ConnectionState::Status,
                                Direction::Serverbound,
                                version,
                            ).unwrap();

                            let payload = match decoded {
                                DecodedPacket::Typed { packet, .. } => {
                                    packet.as_any().downcast_ref::<SPingRequest>()
                                        .map_or(0, |p| p.payload)
                                }
                                DecodedPacket::Opaque { .. } => 0,
                            };

                            // Send CPingResponse
                            let pong = CPingResponse { payload };
                            send_packet(&registry, &mut stream, &pong, version).await.unwrap();
                        });
                    }
                    () = s.cancelled() => break,
                }
            }
        });

        let server_addr: ServerAddress = format!("127.0.0.1:{}", addr.port()).parse().unwrap();

        (vec![server_addr], shutdown)
    }

    fn make_server_config(addresses: Vec<ServerAddress>) -> ServerConfig {
        ServerConfig {
            id: Some("test".to_string()),
            name: None,
            network: None,
            domains: vec!["test.mc".to_string()],
            addresses,
            proxy_mode: Default::default(),
            forwarding_mode: None,
            send_proxy_protocol: false,
            domain_rewrite: DomainRewrite::None,
            motd: Default::default(),
            server_manager: None,
            timeouts: None,
            max_players: 0,
            ip_filter: None,
            disconnect_message: None,
            limbo_handlers: vec![],
        }
    }

    fn make_client_info() -> ConnectionInfo {
        ConnectionInfo {
            peer_addr: "127.0.0.1:12345".parse().unwrap(),
            real_ip: None,
            real_port: None,
            local_addr: "127.0.0.1:25565".parse().unwrap(),
            connected_at: tokio::time::Instant::now(),
        }
    }

    #[tokio::test]
    async fn test_relay_success() {
        let (addrs, shutdown) = spawn_mock_mc_status(TEST_JSON).await;
        let server_config = make_server_config(addrs);

        let connector = Arc::new(BackendConnector::new(
            Duration::from_secs(5),
            KeepaliveConfig::default(),
        ));
        let registry = Arc::new(build_default_registry());
        let client = StatusRelayClient::new(connector, registry, Duration::from_secs(5));

        let result = client
            .relay(
                "test",
                &server_config,
                "test.mc",
                ProtocolVersion::V1_21,
                &make_client_info(),
            )
            .await
            .unwrap();

        assert_eq!(result.response.players.online, 42);
        assert_eq!(result.response.players.max, 100);
        assert!(result.latency < Duration::from_secs(1));

        shutdown.cancel();
    }

    #[tokio::test]
    async fn test_relay_timeout() {
        // Bind a listener that never accepts
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Accept but never respond
        tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            // Hold the connection open but never send data
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        let server_addr: ServerAddress = format!("127.0.0.1:{}", addr.port()).parse().unwrap();
        let server_config = make_server_config(vec![server_addr]);

        let connector = Arc::new(BackendConnector::new(
            Duration::from_secs(5),
            KeepaliveConfig::default(),
        ));
        let registry = Arc::new(build_default_registry());
        let client = StatusRelayClient::new(connector, registry, Duration::from_millis(200));

        let result = client
            .relay(
                "test",
                &server_config,
                "test.mc",
                ProtocolVersion::V1_21,
                &make_client_info(),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("timeout"),
            "expected timeout error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_relay_connection_refused() {
        // Use a port that no one is listening on
        let server_addr: ServerAddress = "127.0.0.1:1".parse().unwrap();
        let server_config = make_server_config(vec![server_addr]);

        let connector = Arc::new(BackendConnector::new(
            Duration::from_secs(1),
            KeepaliveConfig::default(),
        ));
        let registry = Arc::new(build_default_registry());
        let client = StatusRelayClient::new(connector, registry, Duration::from_secs(5));

        let result = client
            .relay(
                "test",
                &server_config,
                "test.mc",
                ProtocolVersion::V1_21,
                &make_client_info(),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_relay_invalid_json() {
        let (addrs, shutdown) = spawn_mock_mc_status("not valid json {{{").await;
        let server_config = make_server_config(addrs);

        let connector = Arc::new(BackendConnector::new(
            Duration::from_secs(5),
            KeepaliveConfig::default(),
        ));
        let registry = Arc::new(build_default_registry());
        let client = StatusRelayClient::new(connector, registry, Duration::from_secs(5));

        let result = client
            .relay(
                "test",
                &server_config,
                "test.mc",
                ProtocolVersion::V1_21,
                &make_client_info(),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid status JSON"), "got: {err}");

        shutdown.cancel();
    }

    #[test]
    fn test_resolve_relay_domain_none() {
        let cfg = make_server_config(vec!["backend:25565".parse().unwrap()]);
        assert_eq!(resolve_relay_domain("play.mc", &cfg), "play.mc");
    }

    #[test]
    fn test_resolve_relay_domain_explicit() {
        let mut cfg = make_server_config(vec!["backend:25565".parse().unwrap()]);
        cfg.domain_rewrite = DomainRewrite::Explicit("custom.host".to_string());
        assert_eq!(resolve_relay_domain("play.mc", &cfg), "custom.host");
    }

    #[test]
    fn test_resolve_relay_domain_from_backend() {
        let mut cfg = make_server_config(vec!["backend.local:25565".parse().unwrap()]);
        cfg.domain_rewrite = DomainRewrite::FromBackend;
        assert_eq!(resolve_relay_domain("play.mc", &cfg), "backend.local");
    }

    #[tokio::test]
    async fn test_relay_with_unsupported_protocol_version() {
        let (addrs, shutdown) = spawn_mock_mc_status(TEST_JSON).await;
        let server_config = make_server_config(addrs);

        let connector = Arc::new(BackendConnector::new(
            Duration::from_secs(5),
            KeepaliveConfig::default(),
        ));
        let registry = Arc::new(build_default_registry());
        let client = StatusRelayClient::new(connector, registry, Duration::from_secs(5));

        let future_version = ProtocolVersion(9999);
        let result = client
            .relay(
                "test",
                &server_config,
                "test.mc",
                future_version,
                &make_client_info(),
            )
            .await
            .unwrap();

        assert_eq!(result.response.players.online, 42);
        assert_eq!(result.response.players.max, 100);

        shutdown.cancel();
    }
}
