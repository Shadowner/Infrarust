use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::AsyncReadExt;

use infrarust_protocol::io::PacketDecoder;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::login::SLoginStart;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{HandshakeData, LoginData};

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
            let login_start = SLoginStart::decode(&mut frame.payload.as_ref(), protocol_version)?;

            tracing::debug!(
                username = %login_start.name,
                uuid = ?login_start.uuid,
                "login start parsed"
            );

            ctx.extensions.insert(LoginData {
                username: login_start.name.clone(),
                player_uuid: login_start.uuid,
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
