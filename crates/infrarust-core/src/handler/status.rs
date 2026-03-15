use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use infrarust_protocol::io::{PacketDecoder, PacketEncoder};
use infrarust_protocol::packets::status::{CPingResponse, CStatusResponse, SPingRequest};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{CURRENT_MC_PROTOCOL, CURRENT_MC_VERSION, Packet};

use infrarust_server_manager::{ServerManagerService, ServerState};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::registry::ConnectionRegistry;

/// Handles Minecraft status ping requests (intent = Status).
///
/// Returns contextual MOTD based on server state when a server_manager is configured.
pub struct StatusHandler {
    registry: Arc<PacketRegistry>,
    server_manager: Option<Arc<ServerManagerService>>,
}

impl StatusHandler {
    /// Creates a new status handler.
    pub fn new(
        registry: Arc<PacketRegistry>,
        server_manager: Option<Arc<ServerManagerService>>,
    ) -> Self {
        Self {
            registry,
            server_manager,
        }
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
            .map_or(ProtocolVersion(CURRENT_MC_PROTOCOL), |h| h.protocol_version);

        // Wait for SStatusRequest packet (empty packet, just the frame)
        let mut decoder = PacketDecoder::new();

        loop {
            if decoder.try_next_frame()?.is_some() {
                break; // Got the status request
            }
            let mut buf = [0u8; 512];
            let n = ctx.stream_mut().read(&mut buf).await?;
            if n == 0 {
                return Err(CoreError::ConnectionClosed);
            }
            decoder.queue_bytes(&buf[..n]);
        }

        // Build and send status response
        let json = Self::build_status_json(
            routing.as_ref(),
            connection_registry,
            self.server_manager.as_deref(),
        );
        let response = CStatusResponse {
            json_response: json,
        };

        self.send_packet(ctx, &response, protocol_version).await?;

        // Wait for ping request
        let mut decoder = PacketDecoder::new();
        let frame = loop {
            if let Some(frame) = decoder.try_next_frame()? {
                break frame;
            }
            let mut buf = [0u8; 512];
            let n = ctx.stream_mut().read(&mut buf).await?;
            if n == 0 {
                return Err(CoreError::ConnectionClosed);
            }
            decoder.queue_bytes(&buf[..n]);
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
                .map_or(0, |p| p.payload),
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

    /// Resolves the MOTD text based on server state.
    fn resolve_motd(
        cfg: &infrarust_config::ServerConfig,
        server_manager: Option<&ServerManagerService>,
    ) -> String {
        // If no server_manager service or no server_manager config → use online MOTD
        if cfg.server_manager.is_none() {
            return cfg
                .motd
                .online
                .as_ref()
                .map_or("An Infrarust Proxy".to_string(), |m| m.text.clone());
        }

        let server_id = cfg.effective_id();
        let state = server_manager.and_then(|sm| sm.get_state(&server_id));

        match state {
            Some(ServerState::Online) => cfg
                .motd
                .online
                .as_ref()
                .map_or("A Minecraft Server".to_string(), |m| m.text.clone()),
            Some(ServerState::Sleeping) => cfg.motd.sleeping.as_ref().map_or(
                "\u{00a7}7Server sleeping \u{2014} \u{00a7}aConnect to wake up!".to_string(),
                |m| m.text.clone(),
            ),
            Some(ServerState::Starting) => cfg
                .motd
                .starting
                .as_ref()
                .map_or("\u{00a7}eServer is starting...".to_string(), |m| {
                    m.text.clone()
                }),
            Some(ServerState::Crashed) => cfg
                .motd
                .crashed
                .as_ref()
                .map_or("\u{00a7}cServer unavailable".to_string(), |m| {
                    m.text.clone()
                }),
            _ => "A Minecraft Server".to_string(),
        }
    }

    fn build_status_json(
        routing: Option<&RoutingData>,
        connection_registry: &ConnectionRegistry,
        server_manager: Option<&ServerManagerService>,
    ) -> String {
        let (motd_text, max_players, version_name, _favicon) = routing.map_or_else(
            || ("An Infrarust Proxy".to_string(), 0u32, None, None),
            |rd| {
                let cfg = &rd.server_config;

                // Determine MOTD based on server state if a server_manager is active
                let motd_text = Self::resolve_motd(cfg, server_manager);

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
                (motd_text, max, ver, favicon)
            },
        );

        let online = routing.map_or(0, |rd| connection_registry.count_by_server(&rd.config_id));

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
