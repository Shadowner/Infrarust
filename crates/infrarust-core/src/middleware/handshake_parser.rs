use std::future::Future;
use std::pin::Pin;

use tokio::io::AsyncReadExt;

use infrarust_protocol::io::PacketDecoder;
use infrarust_protocol::legacy;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{ConnectionIntent, HandshakeData, LegacyDetected};

/// Middleware that parses the Minecraft handshake packet.
///
/// Detects legacy clients (0xFE first byte) and short-circuits.
/// For modern clients, decodes the SHandshake packet directly (without the
/// packet registry), strips FML markers, and inserts `HandshakeData` into
/// the context extensions.
///
/// The handshake format has been stable since Minecraft 1.7 and is decoded
/// independently of the client's protocol version. This makes the proxy
/// forward-compatible with any future Minecraft version.
#[derive(Default)]
pub struct HandshakeParserMiddleware;

impl HandshakeParserMiddleware {
    pub fn new() -> Self {
        Self
    }
}

/// Strips Forge Mod Loader markers from the domain string.
fn strip_fml_markers(domain: &str) -> &str {
    // FML markers: \0FML\0, \0FML2\0, \0FML3\0
    if let Some(pos) = domain.find('\0') {
        &domain[..pos]
    } else {
        domain
    }
}

impl Middleware for HandshakeParserMiddleware {
    fn name(&self) -> &'static str {
        "handshake_parser"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            // Read first byte to detect legacy vs modern
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

            // Legacy detection
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

            // Modern handshake: read enough data to decode
            // Keep reading until we can decode a full packet frame
            let mut decoder = PacketDecoder::new();
            decoder.queue_bytes(&ctx.buffered_data);

            let mut raw_data = ctx.buffered_data.clone();
            let frame = loop {
                match decoder.try_next_frame()? {
                    Some(frame) => break frame,
                    None => {
                        // Need more data from the stream
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

            // Store all raw bytes read so far for forwarding
            let raw_packet = raw_data;

            // Decode the handshake directly — always packet ID 0x00,
            // format stable since MC 1.7, no registry needed.
            if frame.id != 0x00 {
                return Err(CoreError::Protocol(
                    infrarust_protocol::ProtocolError::invalid(format!(
                        "expected handshake (0x00), got 0x{:02X}",
                        frame.id,
                    )),
                ));
            }
            let handshake = SHandshake::decode(
                &mut frame.payload.as_ref(),
                ProtocolVersion::V1_7_2,
            )?;

            // Extract and clean domain
            let domain = strip_fml_markers(&handshake.server_address).to_lowercase();
            let port = handshake.server_port;
            let protocol_version = ProtocolVersion(handshake.protocol_version.0);

            // Map next_state to ConnectionIntent
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
                raw_packets: vec![raw_packet],
            });

            // Update buffered_data to only contain unprocessed bytes
            ctx.buffered_data.clear();

            Ok(MiddlewareResult::Continue)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_fml_markers() {
        assert_eq!(strip_fml_markers("mc.example.com"), "mc.example.com");
        assert_eq!(strip_fml_markers("mc.example.com\0FML\0"), "mc.example.com");
        assert_eq!(
            strip_fml_markers("mc.example.com\0FML2\0"),
            "mc.example.com"
        );
        assert_eq!(
            strip_fml_markers("mc.example.com\0FML3\0"),
            "mc.example.com"
        );
    }
}
