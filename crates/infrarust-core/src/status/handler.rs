//! Refactored status handler with relay, cache, and contextual MOTDs.
//!
//! Replaces the Phase 1/2C handler (`handler/status.rs`). Implements the
//! full decision tree: server manager states → relay → cache → stale
//! fallback → synthetic MOTDs.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex as TokioMutex;

use infrarust_api::events::proxy::ProxyPingEvent;
use infrarust_config::{MotdConfig, ServerConfig};
use infrarust_protocol::io::{PacketDecoder, PacketEncoder};
use infrarust_protocol::packets::status::{CPingResponse, CStatusResponse, SPingRequest};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{CURRENT_MC_PROTOCOL, Packet};

use infrarust_server_manager::{ServerManagerService, ServerState};

use super::cache::StatusCache;
use super::favicon::FaviconCache;
use super::relay::StatusRelayClient;
use super::response::ServerPingResponse;
use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::event_bus::conversion::{apply_api_to_core, core_to_api_ping_response};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::registry::ConnectionRegistry;

/// Handles modern (1.7+) status pings with relay, cache, and contextual MOTDs.
pub struct StatusHandler {
    relay_client: StatusRelayClient,
    cache: Arc<StatusCache>,
    favicon_cache: Arc<FaviconCache>,
    server_manager: Option<Arc<ServerManagerService>>,
    registry: Arc<PacketRegistry>,
    default_motd: Option<MotdConfig>,
    event_bus: Arc<EventBusImpl>,
    /// Only one relay runs at a time per server; concurrent requests wait.
    inflight_locks: DashMap<String, Arc<TokioMutex<()>>>,
}

impl StatusHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        relay_client: StatusRelayClient,
        cache: Arc<StatusCache>,
        favicon_cache: Arc<FaviconCache>,
        server_manager: Option<Arc<ServerManagerService>>,
        registry: Arc<PacketRegistry>,
        default_motd: Option<MotdConfig>,
        event_bus: Arc<EventBusImpl>,
    ) -> Self {
        Self {
            relay_client,
            cache,
            favicon_cache,
            server_manager,
            registry,
            default_motd,
            event_bus,
            inflight_locks: DashMap::new(),
        }
    }

    /// Returns a reference to the status cache (for hot-reload invalidation).
    pub const fn cache(&self) -> &Arc<StatusCache> {
        &self.cache
    }

    /// Returns a reference to the favicon cache (for hot-reload).
    pub const fn favicon_cache(&self) -> &Arc<FaviconCache> {
        &self.favicon_cache
    }

    /// Handles a status request on the given connection context.
    ///
    /// # Errors
    /// Returns `CoreError` on I/O or protocol errors during the status exchange.
    #[tracing::instrument(name = "status.ping", skip_all)]
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

        self.read_status_request(ctx).await?;

        let mut response = self
            .resolve_response(
                ctx,
                routing.as_ref(),
                handshake.as_ref(),
                connection_registry,
            )
            .await;

        let api_response = core_to_api_ping_response(&response);
        let remote_addr = SocketAddr::new(ctx.client_ip, ctx.peer_addr.port());
        let event = ProxyPingEvent {
            remote_addr,
            response: api_response,
        };
        let event = self.event_bus.fire(event).await;
        apply_api_to_core(&mut response, &event.response);

        let json = response
            .to_json()
            .map_err(|e| CoreError::Other(format!("failed to serialize status JSON: {e}")))?;
        let status_resp = CStatusResponse {
            json_response: json,
        };
        self.send_packet(ctx, &status_resp, protocol_version)
            .await?;

        // Skip ping/pong for offline servers so the client shows an X on the bars
        if response.version.protocol != -1 {
            self.handle_ping_pong(ctx, protocol_version).await?;
        }

        Ok(())
    }

    /// Resolves the status response based on the decision tree.
    async fn resolve_response(
        &self,
        ctx: &ConnectionContext,
        routing: Option<&RoutingData>,
        handshake: Option<&HandshakeData>,
        connection_registry: &ConnectionRegistry,
    ) -> ServerPingResponse {
        let Some(routing) = routing else {
            return self.build_default_motd_response();
        };

        let config = &routing.server_config;
        let config_id = &routing.config_id;

        if config.server_manager.is_some()
            && let Some(ref sm) = self.server_manager
        {
            match sm.get_state(config_id) {
                Some(ServerState::Online) | None => {}
                Some(state) => {
                    return Self::build_state_motd(config, state);
                }
            }
        }

        let mut response = self
            .relay_or_cache(ctx, config, config_id, handshake, connection_registry)
            .await;

        if let Some(ref online) = config.motd.online {
            response.apply_overrides(online);
        }

        if response.favicon.is_none()
            && let Some(fav) = self.favicon_cache.get(config_id)
        {
            response.favicon = Some(fav);
        }

        response
    }

    /// Attempts relay → cache → stale → synthetic fallback.
    ///
    /// Uses per-server locking to prevent cache stampede: only one relay
    /// runs at a time per server, concurrent requests wait and re-check cache.
    async fn relay_or_cache(
        &self,
        ctx: &ConnectionContext,
        config: &ServerConfig,
        config_id: &str,
        handshake: Option<&HandshakeData>,
        connection_registry: &ConnectionRegistry,
    ) -> ServerPingResponse {
        if let Some((response, _latency)) = self.cache.get_fresh(config_id) {
            return response;
        }

        let lock = self
            .inflight_locks
            .entry(config_id.to_string())
            .or_insert_with(|| Arc::new(TokioMutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        if let Some((response, _latency)) = self.cache.get_fresh(config_id) {
            return response;
        }

        let domain = handshake.map_or("localhost", |h| h.domain.as_str());
        let protocol_version =
            handshake.map_or(ProtocolVersion(CURRENT_MC_PROTOCOL), |h| h.protocol_version);
        let client_info = ctx.connection_info();

        match self
            .relay_client
            .relay(config_id, config, domain, protocol_version, &client_info)
            .await
        {
            Ok(result) => {
                self.cache
                    .put(config_id, result.response.clone(), result.latency, None);
                return result.response;
            }
            Err(e) => {
                tracing::debug!(
                    server = config_id,
                    error = %e,
                    "status relay failed, trying cache fallback"
                );
            }
        }

        if let Some((mut response, _latency)) = self.cache.get_stale(config_id) {
            tracing::warn!(
                server = config_id,
                "serving stale cached status (backend unreachable)"
            );
            let unreachable_entry = config.motd.unreachable.as_ref().or_else(|| {
                self.default_motd
                    .as_ref()
                    .and_then(|m| m.unreachable.as_ref())
            });
            if let Some(entry) = unreachable_entry {
                response.apply_overrides(entry);
            }
            return response;
        }

        self.build_unreachable_motd(config, connection_registry, config_id)
    }

    /// Builds a synthetic MOTD for the given server manager state.
    fn build_state_motd(config: &ServerConfig, state: ServerState) -> ServerPingResponse {
        let (motd_entry, default_text) = match state {
            ServerState::Sleeping => (
                config.motd.sleeping.as_ref(),
                "\u{00a7}7Server sleeping \u{2014} \u{00a7}aConnect to wake up!",
            ),
            ServerState::Starting => (
                config.motd.starting.as_ref(),
                "\u{00a7}eServer is starting...",
            ),
            ServerState::Crashed => (config.motd.crashed.as_ref(), "\u{00a7}cServer unavailable"),
            ServerState::Stopping => (
                config.motd.stopping.as_ref(),
                "\u{00a7}6Server is stopping...",
            ),
            _ => (None, "A Minecraft Server"),
        };

        let mut resp = motd_entry.map_or_else(
            || ServerPingResponse::synthetic(default_text, None, None, None),
            |entry| {
                let mut r = ServerPingResponse::synthetic(
                    &entry.text,
                    entry.favicon.as_deref(),
                    entry.version_name.as_deref(),
                    entry.max_players.map(u32::cast_signed),
                );
                if let Some(protocol) = entry.version_protocol {
                    r.version.protocol = protocol;
                }
                r
            },
        );
        resp
    }

    /// Builds a response from the global `default_motd` (unknown domain).
    fn build_default_motd_response(&self) -> ServerPingResponse {
        let entry = self.default_motd.as_ref().and_then(|m| m.online.as_ref());

        let mut resp = entry.map_or_else(
            || ServerPingResponse::synthetic("An Infrarust Proxy", None, None, None),
            |entry| {
                ServerPingResponse::synthetic(
                    &entry.text,
                    entry.favicon.as_deref(),
                    entry.version_name.as_deref(),
                    entry.max_players.map(u32::cast_signed),
                )
            },
        );
        if let Some(protocol) = entry.and_then(|e| e.version_protocol) {
            resp.version.protocol = protocol;
        }
        resp
    }

    /// Builds a synthetic "unreachable" MOTD.
    fn build_unreachable_motd(
        &self,
        config: &ServerConfig,
        connection_registry: &ConnectionRegistry,
        config_id: &str,
    ) -> ServerPingResponse {
        let motd_entry = config.motd.unreachable.as_ref().or_else(|| {
            self.default_motd
                .as_ref()
                .and_then(|m| m.unreachable.as_ref())
        });

        let mut resp = motd_entry.map_or_else(
            || {
                ServerPingResponse::synthetic(
                    "\u{00a7}cServer unreachable",
                    None,
                    None,
                    Some(config.max_players.cast_signed()),
                )
            },
            |entry| {
                ServerPingResponse::synthetic(
                    &entry.text,
                    entry.favicon.as_deref(),
                    entry.version_name.as_deref(),
                    entry.max_players.map(u32::cast_signed),
                )
            },
        );
        if let Some(protocol) = motd_entry.and_then(|e| e.version_protocol) {
            resp.version.protocol = protocol;
        }
        let online = connection_registry.count_by_server(config_id) as i32;
        resp.players.online = online;
        resp
    }

    /// Reads the `SStatusRequest` frame from the client (with timeout).
    async fn read_status_request(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut decoder = PacketDecoder::new();
            if !ctx.buffered_data.is_empty() {
                decoder.queue_bytes(&ctx.buffered_data);
                ctx.buffered_data.clear();
            }
            loop {
                if decoder.try_next_frame()?.is_some() {
                    return Ok(());
                }
                let mut buf = [0u8; 512];
                let n = ctx.stream_mut().read(&mut buf).await?;
                if n == 0 {
                    return Err(CoreError::ConnectionClosed);
                }
                decoder.queue_bytes(&buf[..n]);
            }
        })
        .await
        .map_err(|_| CoreError::Timeout("status request read timed out".into()))?
    }

    /// Handles the ping/pong exchange after status response.
    #[allow(clippy::similar_names)] // decoder vs decoded are contextually different
    async fn handle_ping_pong(
        &self,
        ctx: &mut ConnectionContext,
        protocol_version: ProtocolVersion,
    ) -> Result<(), CoreError> {
        let frame = tokio::time::timeout(Duration::from_secs(5), async {
            let mut decoder = PacketDecoder::new();
            loop {
                if let Some(frame) = decoder.try_next_frame()? {
                    return Ok(frame);
                }
                let mut buf = [0u8; 512];
                let n = ctx.stream_mut().read(&mut buf).await?;
                if n == 0 {
                    return Err(CoreError::ConnectionClosed);
                }
                decoder.queue_bytes(&buf[..n]);
            }
        })
        .await
        .map_err(|_| CoreError::Timeout("ping request read timed out".into()))??;

        let decoded = self.registry.decode_frame(
            &frame,
            ConnectionState::Status,
            Direction::Serverbound,
            protocol_version,
        )?;

        let payload = match decoded {
            DecodedPacket::Typed { packet, .. } => packet
                .as_any()
                .downcast_ref::<SPingRequest>()
                .map_or(0, |p| p.payload),
            DecodedPacket::Opaque { .. } => 0,
        };

        let pong = CPingResponse { payload };
        self.send_packet(ctx, &pong, protocol_version).await
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
            .unwrap_or(0);

        let mut payload = Vec::new();
        packet.encode(&mut payload, version)?;

        let mut encoder = PacketEncoder::new();
        encoder.append_raw(packet_id, &payload)?;
        let bytes = encoder.take();

        ctx.stream_mut().write_all(&bytes).await?;
        ctx.stream_mut().flush().await?;
        Ok(())
    }
}
