use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::AsyncReadExt;

use infrarust_protocol::io::PacketDecoder;
use infrarust_protocol::legacy;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::handler::legacy::hello;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::preauth_frame::{MAX_HANDSHAKE_FRAME, read_preauth_frame};
use crate::pipeline::types::{ConnectionIntent, HandshakeData};

/// Middleware that parses the Minecraft handshake packet.
///
/// Legacy clients (first byte 0xFE or 0x02) get an ordinary `HandshakeData` next
/// to their `LegacyHello`, so the rest of the pipeline treats them alike.
/// For modern clients, decodes the `SHandshake` packet directly (without the
/// packet registry), strips FML markers, and inserts `HandshakeData` into
/// the context extensions.
///
/// The handshake format has been stable since Minecraft 1.7 and is decoded
/// independently of the client's protocol version. This makes the proxy
/// forward-compatible with any future Minecraft version.
#[derive(Default)]
pub struct HandshakeParserMiddleware;

impl HandshakeParserMiddleware {
    pub const fn new() -> Self {
        Self
    }
}

use crate::util::normalize_handshake;

impl Middleware for HandshakeParserMiddleware {
    fn name(&self) -> &'static str {
        "handshake_parser"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::timeout(Duration::from_secs(10), async {
                let first_byte = if ctx.buffered_data.is_empty() {
                    let mut buf = [0u8; 1];
                    let n = ctx.stream_mut().read(&mut buf).await?;
                    if n == 0 {
                        return Err(CoreError::ConnectionClosed);
                    }
                    ctx.buffered_data.extend_from_slice(&buf[..n]);
                    buf[0]
                } else {
                    ctx.buffered_data[0]
                };

                match legacy::detect(first_byte) {
                    legacy::LegacyDetection::LegacyPing => {
                        hello::read_ping_hello(ctx).await?;
                        return Ok(MiddlewareResult::Continue);
                    }
                    legacy::LegacyDetection::LegacyLogin => {
                        hello::read_login_hello(ctx).await?;
                        return Ok(MiddlewareResult::Continue);
                    }
                    legacy::LegacyDetection::Modern => {}
                }

                let mut decoder = PacketDecoder::new();
                decoder.queue_bytes(&ctx.buffered_data);

                let mut raw_data = ctx.buffered_data.clone();
                let frame = read_preauth_frame(
                    ctx.stream_mut(),
                    &mut decoder,
                    &mut raw_data,
                    MAX_HANDSHAKE_FRAME,
                )
                .await?;

                // Separate handshake bytes from trailing data (e.g. LoginStart, SStatusRequest)
                let remaining = decoder.into_remaining();
                raw_data.truncate(raw_data.len() - remaining.len());
                ctx.buffered_data = remaining;

                if frame.id != 0x00 {
                    return Err(CoreError::Protocol(
                        infrarust_protocol::ProtocolError::invalid(format!(
                            "expected handshake (0x00), got 0x{:02X}",
                            frame.id,
                        )),
                    ));
                }
                let handshake =
                    SHandshake::decode(&mut frame.payload.as_ref(), ProtocolVersion::V1_7_2)?;

                let domain = normalize_handshake(&handshake.server_address).to_lowercase();
                let port = handshake.server_port;
                let protocol_version = ProtocolVersion(handshake.protocol_version.0);

                let intent = match (handshake.next_state, handshake.transfer) {
                    (ConnectionState::Status, _) => ConnectionIntent::Status,
                    (_, true) => ConnectionIntent::Transfer,
                    (_, false) => ConnectionIntent::Login,
                };

                tracing::debug!(
                    domain = %domain,
                    port,
                    protocol = protocol_version.0,
                    ?intent,
                    "handshake parsed"
                );

                if !protocol_version.is_known() {
                    tracing::warn!(
                        protocol = protocol_version.0,
                        inherits = %ProtocolVersion::HIGHEST_KNOWN,
                        "client protocol is newer than any version this build knows;                          play and config packet ids are inherited and may be wrong"
                    );
                }

                ctx.extensions.insert(HandshakeData {
                    domain,
                    raw_host: handshake.server_address,
                    port,
                    protocol_version,
                    intent,
                    raw_packets: vec![raw_data],
                });

                Ok(MiddlewareResult::Continue)
            })
            .await
            .map_err(|_| CoreError::Timeout("handshake read timed out".into()))?
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::net::{IpAddr, Ipv4Addr};

    use infrarust_protocol::ProtocolError;
    use infrarust_protocol::codec::VarInt;
    use infrarust_protocol::io::PacketEncoder;
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    async fn ctx_with(wire: &[u8]) -> (ConnectionContext, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, peer) = listener.accept().await.unwrap();
        let mut ctx =
            ConnectionContext::new_for_test(server, peer, IpAddr::V4(Ipv4Addr::LOCALHOST), addr);
        ctx.buffered_data.extend_from_slice(wire);
        (ctx, client)
    }

    fn handshake_frame(host: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        SHandshake {
            protocol_version: VarInt(ProtocolVersion::V1_21.0),
            server_address: host.to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
            transfer: false,
        }
        .encode(&mut payload, ProtocolVersion::V1_7_2)
        .unwrap();
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, &payload).unwrap();
        encoder.take().to_vec()
    }

    #[tokio::test]
    async fn an_oversized_handshake_is_refused_from_its_declared_length() {
        let mut declared = Vec::new();
        VarInt(2049).encode(&mut declared).unwrap();
        let (mut ctx, _client) = ctx_with(&declared).await;

        let refused = tokio::time::timeout(
            Duration::from_secs(1),
            HandshakeParserMiddleware::new().process(&mut ctx),
        )
        .await
        .expect("the proxy must not wait for the declared bytes");

        assert!(matches!(
            refused,
            Err(CoreError::Protocol(ProtocolError::TooLarge { .. }))
        ));
    }

    #[tokio::test]
    async fn bytes_behind_a_handshake_are_left_for_the_next_stage() {
        let frame = handshake_frame("mc.example.com");
        let mut wire = frame.clone();
        wire.extend_from_slice(&[0xAB; 8192]);
        let (mut ctx, _client) = ctx_with(&wire).await;

        let result = HandshakeParserMiddleware::new().process(&mut ctx).await;

        assert!(matches!(result.unwrap(), MiddlewareResult::Continue));
        let handshake = ctx.extensions.get::<HandshakeData>().unwrap();
        assert_eq!(handshake.domain, "mc.example.com");
        assert_eq!(handshake.raw_packets[0].as_ref(), frame.as_slice());
        assert_eq!(ctx.buffered_data.len(), 8192);
    }

    #[test]
    fn test_normalize_handshake() {
        assert_eq!(normalize_handshake("mc.example.com"), "mc.example.com");
        assert_eq!(
            normalize_handshake("mc.example.com\0FML\0"),
            "mc.example.com"
        );
        assert_eq!(
            normalize_handshake("mc.example.com\0FML2\0"),
            "mc.example.com"
        );
        assert_eq!(
            normalize_handshake("mc.example.com\0FML3\0"),
            "mc.example.com"
        );
        // SRV-resolved FQDN with trailing dot (MC-41034) — login flow only.
        assert_eq!(normalize_handshake("mc.example.com."), "mc.example.com");
        assert_eq!(normalize_handshake("mc.example.com..."), "mc.example.com");
        assert_eq!(
            normalize_handshake("mc.example.com.\0FML2\0"),
            "mc.example.com"
        );
    }
}
