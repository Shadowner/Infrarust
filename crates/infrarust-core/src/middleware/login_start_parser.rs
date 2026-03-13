use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::io::AsyncReadExt;

use infrarust_protocol::io::PacketDecoder;
use infrarust_protocol::packets::login::SLoginStart;
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{HandshakeData, LoginData};

/// Middleware that parses the LoginStart packet to extract username and UUID.
///
/// Runs in the login pipeline after the common pipeline has completed.
/// Appends the raw login packet bytes to `HandshakeData.raw_packets`
/// for forwarding to the backend in passthrough mode.
pub struct LoginStartParserMiddleware {
    registry: Arc<PacketRegistry>,
}

impl LoginStartParserMiddleware {
    /// Creates a new login start parser using the given packet registry.
    pub fn new(registry: Arc<PacketRegistry>) -> Self {
        Self { registry }
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
                .extensions
                .get::<HandshakeData>()
                .expect("HandshakeData must be set by handshake_parser")
                .protocol_version;

            // Read the login start packet
            let mut decoder = PacketDecoder::new();

            // Feed any buffered data first
            if !ctx.buffered_data.is_empty() {
                decoder.queue_bytes(&ctx.buffered_data);
            }

            let mut raw_data = ctx.buffered_data.clone();
            let frame = loop {
                match decoder.try_next_frame()? {
                    Some(frame) => break frame,
                    None => {
                        let mut buf = [0u8; 1024];
                        let n = ctx.stream_mut().read(&mut buf).await?;
                        if n == 0 {
                            return Err(CoreError::ConnectionClosed);
                        }
                        decoder.queue_bytes(&buf[..n]);
                        raw_data.extend_from_slice(&buf[..n]);
                    }
                }
            };

            // Decode the login start packet
            let decoded = self.registry.decode_frame(
                &frame,
                ConnectionState::Login,
                Direction::Serverbound,
                protocol_version,
            )?;

            let login_start: SLoginStart = match decoded {
                DecodedPacket::Typed { packet, .. } => {
                    match packet.as_any().downcast_ref::<SLoginStart>() {
                        Some(ls) => ls.clone(),
                        None => {
                            return Err(CoreError::Protocol(
                                infrarust_protocol::ProtocolError::invalid(
                                    "expected SLoginStart packet",
                                ),
                            ));
                        }
                    }
                }
                DecodedPacket::Opaque { .. } => {
                    return Err(CoreError::Protocol(
                        infrarust_protocol::ProtocolError::invalid(
                            "login start packet not registered in registry",
                        ),
                    ));
                }
            };

            tracing::debug!(
                username = %login_start.name,
                uuid = ?login_start.uuid,
                "login start parsed"
            );

            ctx.extensions.insert(LoginData {
                username: login_start.name.clone(),
                player_uuid: login_start.uuid,
            });

            // Append raw login packet bytes for passthrough forwarding
            if let Some(handshake) = ctx.extensions.get_mut::<HandshakeData>() {
                handshake.raw_packets.push(raw_data);
            }

            ctx.buffered_data.clear();

            Ok(MiddlewareResult::Continue)
        })
    }
}
