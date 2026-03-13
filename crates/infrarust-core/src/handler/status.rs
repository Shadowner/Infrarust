use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use infrarust_protocol::io::{PacketDecoder, PacketEncoder};
use infrarust_protocol::packets::status::{CPingResponse, CStatusResponse, SPingRequest};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{CURRENT_MC_PROTOCOL, CURRENT_MC_VERSION, Packet};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::registry::ConnectionRegistry;

/// Handles Minecraft status ping requests (intent = Status).
///
/// Phase 1: returns MOTD from config only (no relay backend).
pub struct StatusHandler {
    registry: Arc<PacketRegistry>,
}

impl StatusHandler {
    /// Creates a new status handler.
    pub const fn new(registry: Arc<PacketRegistry>) -> Self {
        Self { registry }
    }

    /// Handles a status request on the given connection context.
    pub async fn handle(
        &self,
        ctx: &mut ConnectionContext,
        connection_registry: &ConnectionRegistry,
    ) -> Result<(), CoreError> {
        let routing = ctx.extensions.get::<RoutingData>().cloned();
        let handshake = ctx.extensions.get::<HandshakeData>().cloned();

        let protocol_version = handshake
            .as_ref()
            .map(|h| h.protocol_version)
            .unwrap_or(ProtocolVersion(CURRENT_MC_PROTOCOL));

        // Wait for SStatusRequest packet (empty packet, just the frame)
        let mut decoder = PacketDecoder::new();

        loop {
            match decoder.try_next_frame()? {
                Some(_frame) => break, // Got the status request
                None => {
                    let mut buf = [0u8; 512];
                    let n = ctx.stream_mut().read(&mut buf).await?;
                    if n == 0 {
                        return Err(CoreError::ConnectionClosed);
                    }
                    decoder.queue_bytes(&buf[..n]);
                }
            }
        }

        // Build and send status response
        let json = self.build_status_json(routing.as_ref(), connection_registry);
        let response = CStatusResponse {
            json_response: json,
        };

        self.send_packet(ctx, &response, protocol_version).await?;

        // Wait for ping request
        let mut decoder = PacketDecoder::new();
        let frame = loop {
            match decoder.try_next_frame()? {
                Some(frame) => break frame,
                None => {
                    let mut buf = [0u8; 512];
                    let n = ctx.stream_mut().read(&mut buf).await?;
                    if n == 0 {
                        return Err(CoreError::ConnectionClosed);
                    }
                    decoder.queue_bytes(&buf[..n]);
                }
            }
        };

        // Decode and echo back as pong
        let ping_decoded = self.registry.decode_frame(
            &frame,
            ConnectionState::Status,
            Direction::Serverbound,
            protocol_version,
        )?;

        let payload = match ping_decoded {
            DecodedPacket::Typed { packet, .. } => packet
                .as_any()
                .downcast_ref::<SPingRequest>()
                .map(|p| p.payload)
                .unwrap_or(0),
            DecodedPacket::Opaque { .. } => 0,
        };

        let pong = CPingResponse { payload };
        self.send_packet(ctx, &pong, protocol_version).await?;

        Ok(())
    }

    /// Encodes and sends a typed packet to the client stream.
    async fn send_packet<P: Packet>(
        &self,
        ctx: &mut ConnectionContext,
        packet: &P,
        version: ProtocolVersion,
    ) -> Result<(), CoreError> {
        let packet_id = self
            .registry
            .get_packet_id::<P>(P::state(), P::direction(), version)
            .unwrap_or(0); // Fallback to 0 for status/ping which are always 0x00

        let mut payload_buf = Vec::new();
        packet.encode(&mut payload_buf, version)?;

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(packet_id, &payload_buf)?;
        let bytes = encoder.take();

        ctx.stream_mut().write_all(&bytes).await?;
        ctx.stream_mut().flush().await?;

        Ok(())
    }

    fn build_status_json(
        &self,
        routing: Option<&RoutingData>,
        connection_registry: &ConnectionRegistry,
    ) -> String {
        let (motd_text, max_players, version_name, _favicon) = match routing {
            Some(rd) => {
                let cfg = &rd.server_config;
                let motd = cfg
                    .motd
                    .online
                    .as_ref()
                    .map(|m| m.text.as_str())
                    .unwrap_or("An Infrarust Proxy");
                let max = cfg
                    .motd
                    .online
                    .as_ref()
                    .and_then(|m| m.max_players)
                    .unwrap_or(cfg.max_players);
                let ver = cfg
                    .motd
                    .online
                    .as_ref()
                    .and_then(|m| m.version_name.clone());
                let favicon = cfg.motd.online.as_ref().and_then(|m| m.favicon.clone());
                (motd.to_string(), max, ver, favicon)
            }
            None => ("An Infrarust Proxy".to_string(), 0u32, None, None),
        };

        let online = routing
            .map(|rd| connection_registry.count_by_server(&rd.config_id))
            .unwrap_or(0);

        let version_name = version_name.unwrap_or_else(|| CURRENT_MC_VERSION.to_string());

        let json = serde_json::json!({
            "version": {
                "name": version_name,
                "protocol": CURRENT_MC_PROTOCOL,
            },
            "players": {
                "max": max_players,
                "online": online,
                "sample": [],
            },
            "description": {
                "text": motd_text,
            },
        });

        json.to_string()
    }
}
