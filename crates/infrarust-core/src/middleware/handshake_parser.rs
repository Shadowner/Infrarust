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
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{ConnectionIntent, HandshakeData, LegacyDetected};

/// Middleware that parses the Minecraft handshake packet.
///
/// Detects legacy clients (0xFE first byte) and short-circuits.
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
                        tracing::debug!("legacy ping detected (0xFE)");
                        ctx.extensions.insert(LegacyDetected);
                        return Ok(MiddlewareResult::ShortCircuit);
                    }
                    legacy::LegacyDetection::LegacyLogin => {
                        tracing::debug!("legacy login detected (0x02) — unsupported");
                        ctx.extensions.insert(LegacyDetected);
                        return Ok(MiddlewareResult::ShortCircuit);
                    }
                    legacy::LegacyDetection::Modern => {}
                }

                let mut decoder = PacketDecoder::new();
                decoder.queue_bytes(&ctx.buffered_data);

                let mut raw_data = ctx.buffered_data.clone();
                let frame = loop {
                    if let Some(frame) = decoder.try_next_frame()? {
                        break frame;
                    }
                    let mut buf = [0u8; 1024];
                    let n = ctx.stream_mut().read(&mut buf).await?;
                    if n == 0 {
                        return Err(CoreError::ConnectionClosed);
                    }
                    decoder.queue_bytes(&buf[..n]);
                    raw_data.extend_from_slice(&buf[..n]);
                };

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

                let intent = match handshake.next_state {
                    ConnectionState::Status => ConnectionIntent::Status,
                    ConnectionState::Login => ConnectionIntent::Login,
                    _ => ConnectionIntent::Transfer,
                };

                tracing::debug!(
                    domain = %domain,
                    port,
                    protocol = protocol_version.0,
                    ?intent,
                    "handshake parsed"
                );

                ctx.extensions.insert(HandshakeData {
                    domain,
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
    use super::*;

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
