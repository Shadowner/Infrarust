use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::AsyncReadExt;

use infrarust_protocol::ProtocolVersion;
use infrarust_protocol::io::PacketDecoder;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::login::SLoginStart;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{HandshakeData, LoginData, LoginStartLayout, RoutingData};

/// Middleware that parses the `LoginStart` packet to extract username and UUID.
///
/// Decodes the packet directly (without the packet registry) for
/// forward-compatibility with unknown protocol versions.
///
/// Runs in the login pipeline after the common pipeline has completed.
/// Appends the raw login packet bytes to `HandshakeData.raw_packets`
/// for forwarding to the backend in passthrough mode.
///
/// **Requires**: `HandshakeData` (from `HandshakeParserMiddleware`)
/// **Inserts**: `LoginData` (username + optional UUID)
#[derive(Default)]
pub struct LoginStartParserMiddleware;

impl LoginStartParserMiddleware {
    pub const fn new() -> Self {
        Self
    }
}

impl Middleware for LoginStartParserMiddleware {
    fn name(&self) -> &'static str {
        "login_start_parser"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let protocol_version = ctx
                .require_extension::<HandshakeData>("HandshakeData")?
                .protocol_version;

            let mut decoder = PacketDecoder::new();
            if !ctx.buffered_data.is_empty() {
                decoder.queue_bytes(&ctx.buffered_data);
            }

            let mut raw_data = ctx.buffered_data.clone();
            let frame = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Some(frame) = decoder.try_next_frame()? {
                        break Ok::<_, CoreError>(frame);
                    }
                    let mut buf = [0u8; 1024];
                    let n = ctx.stream_mut().read(&mut buf).await?;
                    if n == 0 {
                        return Err(CoreError::ConnectionClosed);
                    }
                    decoder.queue_bytes(&buf[..n]);
                    raw_data.extend_from_slice(&buf[..n]);
                }
            })
            .await
            .map_err(|_| CoreError::ConnectionClosed)??;

            if frame.id != 0x00 {
                return Err(CoreError::Protocol(
                    infrarust_protocol::ProtocolError::invalid(format!(
                        "expected login start (0x00), got 0x{:02X}",
                        frame.id,
                    )),
                ));
            }
            let (login_start, layout) =
                decode_login_start(&frame.payload, protocol_version, routes_to_proxy_login(ctx))?;

            tracing::debug!(
                username = %login_start.name,
                uuid = ?login_start.uuid,
                ?layout,
                "login start parsed"
            );

            ctx.extensions.insert(LoginData {
                username: login_start.name.clone(),
                player_uuid: login_start.uuid,
                profile_key: login_start.profile_key,
                raw_payload: frame.payload.clone(),
                layout,
            });

            let remaining = decoder.into_remaining();
            raw_data.truncate(raw_data.len() - remaining.len());

            if let Some(handshake) = ctx.extensions.get_mut::<HandshakeData>() {
                handshake.raw_packets.push(raw_data);
            }

            ctx.buffered_data = remaining;

            Ok(MiddlewareResult::Continue)
        })
    }
}

fn routes_to_proxy_login(ctx: &ConnectionContext) -> bool {
    ctx.extensions
        .get::<RoutingData>()
        .is_none_or(|routing| routing.server_config.proxy_mode.is_intercepted())
}

fn decode_login_start(
    payload: &[u8],
    version: ProtocolVersion,
    proxy_login: bool,
) -> Result<(SLoginStart, LoginStartLayout), CoreError> {
    let strict = match SLoginStart::decode(&mut &payload[..], version) {
        Ok(login_start) => return Ok((login_start, LoginStartLayout::Declared)),
        Err(error) => error,
    };
    if proxy_login {
        return Err(strict.into());
    }
    let login_start = SLoginStart::decode_lenient(&mut &payload[..]).map_err(|_| strict)?;
    tracing::debug!(
        protocol = version.0,
        username = %login_start.name,
        "login start does not match its protocol layout, forwarding it unchanged"
    );
    Ok((login_start, LoginStartLayout::Mismatched))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    use bytes::BytesMut;
    use infrarust_config::{ProxyMode, ServerConfig};
    use infrarust_protocol::{McBufWriteExt, PacketEncoder};
    use tokio::net::{TcpListener, TcpStream};
    use uuid::Uuid;

    use super::*;
    use crate::pipeline::types::ConnectionIntent;

    const NAME: &str = "k3x9q2m7a1";
    const UUID: Uuid = Uuid::from_u128(0xa7c3_5e10_9b2d_4f6e_8a01_c4d2_e3f4_a5b6);
    const PRE_1_20_2: [ProtocolVersion; 5] = [
        ProtocolVersion::V1_19,
        ProtocolVersion::V1_19_1,
        ProtocolVersion::V1_19_3,
        ProtocolVersion::V1_19_4,
        ProtocolVersion::V1_20,
    ];
    const FORWARDING_MODES: [ProxyMode; 3] = [
        ProxyMode::Passthrough,
        ProxyMode::ZeroCopy,
        ProxyMode::ServerOnly,
    ];

    fn modern_layout() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.write_string(NAME).unwrap();
        payload.write_uuid(&UUID).unwrap();
        payload
    }

    fn framed(payload: &[u8]) -> Vec<u8> {
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, payload).unwrap();
        encoder.take().to_vec()
    }

    fn server_config(mode: ProxyMode) -> ServerConfig {
        let mut config: ServerConfig =
            toml::from_str("domains = [\"lobby.test\"]\naddresses = [\"127.0.0.1:25565\"]\n")
                .unwrap();
        config.proxy_mode = mode;
        config
    }

    async fn login_ctx(
        version: ProtocolVersion,
        mode: Option<ProxyMode>,
        wire: &[u8],
    ) -> (ConnectionContext, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, peer) = listener.accept().await.unwrap();
        let mut ctx =
            ConnectionContext::new_for_test(server, peer, IpAddr::V4(Ipv4Addr::LOCALHOST), addr);
        ctx.buffered_data.extend_from_slice(wire);
        ctx.extensions.insert(HandshakeData {
            domain: "lobby.test".to_string(),
            raw_host: "lobby.test".to_string(),
            port: 25565,
            protocol_version: version,
            intent: ConnectionIntent::Login,
            raw_packets: vec![BytesMut::from(&b"handshake"[..])],
        });
        if let Some(mode) = mode {
            ctx.extensions.insert(RoutingData {
                server_config: Arc::new(server_config(mode)),
                config_id: "lobby".to_string(),
                load_balancer: Arc::new(crate::loadbalancer::FirstAvailable),
            });
        }
        (ctx, client)
    }

    async fn parse(
        version: ProtocolVersion,
        mode: Option<ProxyMode>,
        wire: &[u8],
    ) -> (Result<MiddlewareResult, CoreError>, ConnectionContext) {
        let (mut ctx, _client) = login_ctx(version, mode, wire).await;
        let result = LoginStartParserMiddleware::new().process(&mut ctx).await;
        (result, ctx)
    }

    #[tokio::test]
    async fn a_mismatched_layout_is_parsed_leniently_on_forwarding_routes() {
        let payload = modern_layout();
        let frame = framed(&payload);
        let mut wire = frame.clone();
        wire.extend_from_slice(b"after");
        for version in PRE_1_20_2 {
            for mode in FORWARDING_MODES {
                let (result, ctx) = parse(version, Some(mode), &wire).await;
                assert!(matches!(result.unwrap(), MiddlewareResult::Continue));
                let login = ctx.extensions.get::<LoginData>().unwrap();
                assert_eq!(login.username, NAME);
                assert_eq!(login.player_uuid, Some(UUID));
                assert!(login.profile_key.is_none());
                assert_eq!(login.layout, LoginStartLayout::Mismatched, "{version:?}");
                assert_eq!(login.raw_payload.as_ref(), payload.as_slice());
                let handshake = ctx.extensions.get::<HandshakeData>().unwrap();
                assert_eq!(handshake.raw_packets[1].as_ref(), frame.as_slice());
                assert_eq!(ctx.buffered_data.as_ref(), b"after");
            }
        }
    }

    #[tokio::test]
    async fn a_mismatched_layout_is_refused_where_the_proxy_runs_the_login() {
        let payload = modern_layout();
        let wire = framed(&payload);
        for version in PRE_1_20_2 {
            let strict = SLoginStart::decode(&mut payload.as_slice(), version)
                .unwrap_err()
                .to_string();
            for mode in [Some(ProxyMode::Offline), Some(ProxyMode::ClientOnly), None] {
                let (result, ctx) = parse(version, mode, &wire).await;
                match result {
                    Err(CoreError::Protocol(error)) => assert_eq!(error.to_string(), strict),
                    other => panic!("{version:?} {mode:?} must be refused, got {other:?}"),
                }
                assert!(!ctx.extensions.contains::<LoginData>());
            }
        }
    }

    #[tokio::test]
    async fn a_declared_layout_is_parsed_strictly_on_every_route() {
        let packet = SLoginStart {
            name: "Notch".to_string(),
            uuid: Some(UUID),
            profile_key: None,
        };
        for version in PRE_1_20_2.into_iter().chain([ProtocolVersion::V1_20_2]) {
            let mut payload = Vec::new();
            packet.encode(&mut payload, version).unwrap();
            let wire = framed(&payload);
            for mode in [ProxyMode::Passthrough, ProxyMode::Offline] {
                let (result, ctx) = parse(version, Some(mode), &wire).await;
                assert!(matches!(result.unwrap(), MiddlewareResult::Continue));
                let login = ctx.extensions.get::<LoginData>().unwrap();
                assert_eq!(login.username, "Notch");
                assert_eq!(login.layout, LoginStartLayout::Declared);
                assert_eq!(login.raw_payload.as_ref(), payload.as_slice());
                let expected = version
                    .no_less_than(ProtocolVersion::V1_19_1)
                    .then_some(UUID);
                assert_eq!(login.player_uuid, expected, "{version:?}");
            }
        }
    }

    #[tokio::test]
    async fn garbage_is_refused_on_forwarding_routes_too() {
        let mut oversized = Vec::new();
        oversized.write_string(&"a".repeat(17)).unwrap();
        oversized.write_uuid(&UUID).unwrap();
        let mut empty_name = vec![0x00];
        empty_name.extend_from_slice(UUID.as_bytes());
        for payload in [oversized, empty_name, vec![0x02, 0xC3, 0x28]] {
            let (result, ctx) = parse(
                ProtocolVersion::V1_20,
                Some(ProxyMode::Passthrough),
                &framed(&payload),
            )
            .await;
            assert!(matches!(result, Err(CoreError::Protocol(_))), "{payload:?}");
            assert!(!ctx.extensions.contains::<LoginData>());
        }
    }
}
