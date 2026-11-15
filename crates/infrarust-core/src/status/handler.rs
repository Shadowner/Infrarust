use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use infrarust_api::events::proxy::ProxyPingEvent;
use infrarust_api::types::ServerId;
use infrarust_config::{MotdConfig, ServerConfig};
use infrarust_protocol::io::PacketDecoder;
use infrarust_protocol::packets::status::{CPingResponse, CStatusResponse, SPingRequest};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use infrarust_server_manager::{ServerManagerService, ServerState};

use super::STATUS_PROTOCOL_VERSION;

const STATUS_READ_TIMEOUT: Duration = Duration::from_secs(5);
use super::cache::{FrameSource, StatusCache};
use super::favicon::FaviconCache;
use super::motd::{DEFAULT_PROXY_MOTD, state_max_players, state_motd};
use super::relay::{StatusRelayClient, encode_frame, read_frame_within, send_frame, send_packet};
use super::relay_gate::{RelayGate, RelayTurn};
use super::response::ServerPingResponse;
use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::loadbalancer::{AddressConnectionCount, BackendHealthView, peek_backend_addresses};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::registry::ConnectionRegistry;
use crate::status::ping_event::{core_to_api_ping_response, merge_ping_event};
use crate::util::text::api_version;

enum Backed {
    Cached(Arc<ServerPingResponse>),
    Synthetic(ServerPingResponse),
}

enum Resolved<'a> {
    Ready(ServerPingResponse),
    Cached(Arc<ServerPingResponse>, &'a RoutingData),
}

/// Handles modern (1.7+) status pings with relay, cache, and contextual MOTDs.
pub struct StatusHandler {
    relay_client: StatusRelayClient,
    cache: Arc<StatusCache>,
    favicon_cache: Arc<FaviconCache>,
    server_manager: Option<Arc<ServerManagerService>>,
    registry: Arc<PacketRegistry>,
    default_motd: Option<MotdConfig>,
    event_bus: Arc<EventBusImpl>,
    counts: Arc<dyn AddressConnectionCount>,
    health: Arc<dyn BackendHealthView>,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
    relay_gate: Arc<RelayGate>,
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
        counts: Arc<dyn AddressConnectionCount>,
        health: Arc<dyn BackendHealthView>,
        relay_gate: Arc<RelayGate>,
    ) -> Self {
        Self {
            relay_client,
            cache,
            favicon_cache,
            server_manager,
            registry,
            default_motd,
            event_bus,
            counts,
            health,
            #[cfg(feature = "telemetry")]
            metrics: None,
            relay_gate,
        }
    }

    #[cfg(feature = "telemetry")]
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<crate::telemetry::ProxyMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
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
        let mut decoder = self.read_status_request(ctx).await?;

        let frame = self.status_frame(ctx, connection_registry).await?;
        send_frame(ctx.stream_mut(), &frame).await?;

        match self.handle_ping_pong(ctx, &mut decoder).await {
            Err(CoreError::ConnectionClosed) => Ok(()),
            result => result,
        }
    }

    /// Builds the encoded status response frame for this connection.
    ///
    /// # Errors
    /// Returns `CoreError` if the response cannot be serialized or encoded.
    pub async fn status_frame(
        &self,
        ctx: &ConnectionContext,
        connection_registry: &ConnectionRegistry,
    ) -> Result<Bytes, CoreError> {
        let routing = ctx.extensions.get::<RoutingData>();
        let handshake = ctx.extensions.get::<HandshakeData>();
        let client_version = handshake.map_or(ProtocolVersion::CURRENT, |h| h.protocol_version);

        let resolved = self
            .resolve_response(ctx, routing, handshake, client_version, connection_registry)
            .await;
        let listened = self.event_bus.has_listeners::<ProxyPingEvent>();

        let mut response = match resolved {
            Resolved::Cached(cached, routing) if !listened => {
                let favicon = self.favicon_cache.server_favicon(&routing.config_id);
                let source = FrameSource {
                    config: Arc::clone(&routing.server_config),
                    favicon: favicon.clone(),
                };
                return self.cache.frame(&routing.config_id, &cached, source, || {
                    self.encode(&self.dress(
                        ServerPingResponse::clone(&cached),
                        &routing.server_config,
                        favicon.as_deref(),
                    ))
                });
            }
            Resolved::Cached(cached, routing) => self.dress(
                Arc::unwrap_or_clone(cached),
                &routing.server_config,
                self.favicon_cache
                    .server_favicon(&routing.config_id)
                    .as_deref(),
            ),
            Resolved::Ready(response) => response,
        };
        if !listened {
            return self.encode(&response);
        }

        let favicon = response.favicon.take();
        let mut offered = core_to_api_ping_response(&response);
        let sent = offered.clone();
        offered.favicon = favicon;
        let event = ProxyPingEvent::new(
            ctx.client_addr(),
            routing.map(|r| ServerId::new(r.config_id.clone())),
            handshake.map(|h| h.domain.clone()),
            api_version(client_version),
            false,
            offered,
        );
        let event = self.event_bus.fire(event).await;
        merge_ping_event(
            &mut response,
            &sent,
            event.response,
            api_version(client_version),
        );

        self.encode(&response)
    }

    fn encode(&self, response: &ServerPingResponse) -> Result<Bytes, CoreError> {
        let status = CStatusResponse {
            json_response: response.to_json()?,
        };
        encode_frame(&self.registry, &status, STATUS_PROTOCOL_VERSION)
    }

    fn dress(
        &self,
        mut response: ServerPingResponse,
        config: &ServerConfig,
        server_favicon: Option<&str>,
    ) -> ServerPingResponse {
        if let Some(ref online) = config.motd.online {
            response.apply_overrides(online);
        }
        if let Some(favicon) = server_favicon {
            response.favicon = Some(favicon.to_owned());
        } else if response.favicon.is_none() {
            response.favicon = self.default_favicon();
        }
        response
    }

    fn default_favicon(&self) -> Option<String> {
        self.favicon_cache
            .default_favicon()
            .map(|favicon| favicon.as_ref().to_owned())
    }

    /// Resolves the status response based on the decision tree.
    async fn resolve_response<'a>(
        &self,
        ctx: &ConnectionContext,
        routing: Option<&'a RoutingData>,
        handshake: Option<&HandshakeData>,
        client_version: ProtocolVersion,
        connection_registry: &ConnectionRegistry,
    ) -> Resolved<'a> {
        let Some(routing) = routing else {
            let mut response = self.build_default_motd_response(client_version);
            response.favicon = self.default_favicon();
            return Resolved::Ready(response);
        };

        let config = &routing.server_config;
        let config_id = &routing.config_id;

        if config.server_manager.is_some()
            && let Some(ref sm) = self.server_manager
        {
            match sm.get_state(config_id) {
                Some(ServerState::Online) | None => {}
                Some(state) => {
                    return Resolved::Ready(Self::build_state_motd(config, state, client_version));
                }
            }
        }

        match self
            .relay_or_cache(ctx, routing, handshake, client_version, connection_registry)
            .await
        {
            Backed::Cached(cached) => Resolved::Cached(cached, routing),
            Backed::Synthetic(response) => Resolved::Ready(self.dress(
                response,
                config,
                self.favicon_cache.server_favicon(config_id).as_deref(),
            )),
        }
    }

    /// Attempts relay → cache → stale → synthetic fallback.
    ///
    /// Only one relay runs at a time per server. Other requests wait briefly for
    /// its result and then fall back, so a silent backend cannot queue them up.
    async fn relay_or_cache(
        &self,
        ctx: &ConnectionContext,
        routing: &RoutingData,
        handshake: Option<&HandshakeData>,
        client_version: ProtocolVersion,
        connection_registry: &ConnectionRegistry,
    ) -> Backed {
        let config = &routing.server_config;
        let config_id = routing.config_id.as_str();
        if let Some((response, _latency)) = self.cache.get_fresh(config_id) {
            return Backed::Cached(response);
        }

        let leader = match self.relay_gate.enter(config_id).await {
            RelayTurn::Leader(leader) => leader,
            RelayTurn::Unreachable | RelayTurn::Busy => {
                return self.get_unreachable_motd(
                    config,
                    connection_registry,
                    config_id,
                    client_version,
                    false,
                );
            }
        };

        if let Some((response, _latency)) = self.cache.get_fresh(config_id) {
            return Backed::Cached(response);
        }

        let domain = handshake.map_or("localhost", |h| h.domain.as_str());
        let client_info = ctx.connection_info();

        match self
            .relay_client
            .relay(
                config_id,
                config,
                &peek_backend_addresses(
                    config,
                    routing.load_balancer.as_ref(),
                    self.counts.as_ref(),
                    self.health.as_ref(),
                ),
                domain,
                client_version,
                client_info,
            )
            .await
        {
            Ok(result) => {
                #[cfg(feature = "telemetry")]
                if let Some(ref metrics) = self.metrics {
                    metrics.record_backend_status_latency(
                        result.latency.as_secs_f64(),
                        config_id,
                        &result.address.to_string(),
                    );
                }
                let cached = self
                    .cache
                    .put(config_id, result.response, result.latency, None);
                leader.succeeded();
                return Backed::Cached(cached);
            }
            Err(e) => {
                tracing::debug!(
                    server = config_id,
                    error = %e,
                    "status relay failed, trying cache fallback"
                );
                leader.failed();
            }
        }

        self.get_unreachable_motd(config, connection_registry, config_id, client_version, true)
    }

    pub(crate) fn build_state_motd(
        config: &ServerConfig,
        state: ServerState,
        client_version: ProtocolVersion,
    ) -> ServerPingResponse {
        let (entry, default_text) = state_motd(&config.motd, state);
        let max_players = Some(state_max_players(entry, config));
        entry.map_or_else(
            || ServerPingResponse::synthetic(default_text, client_version, max_players),
            |entry| ServerPingResponse::from_entry(entry, client_version, max_players),
        )
    }

    fn build_default_motd_response(&self, client_version: ProtocolVersion) -> ServerPingResponse {
        let entry = self.default_motd.as_ref().and_then(|m| m.online.as_ref());

        entry.map_or_else(
            || ServerPingResponse::synthetic(DEFAULT_PROXY_MOTD, client_version, None),
            |entry| {
                ServerPingResponse::from_entry(
                    entry,
                    client_version,
                    entry.max_players.map(u32::cast_signed),
                )
            },
        )
    }

    /// Builds a synthetic "unreachable" MOTD.
    fn build_unreachable_motd(
        &self,
        config: &ServerConfig,
        connection_registry: &ConnectionRegistry,
        config_id: &str,
        client_version: ProtocolVersion,
    ) -> ServerPingResponse {
        if let Some(entry) = self
            .default_motd
            .as_ref()
            .and_then(|m| m.unreachable.as_ref())
        {
            return ServerPingResponse::from_entry(
                entry,
                client_version,
                entry.max_players.map(u32::cast_signed),
            );
        }

        let mut resp = ServerPingResponse::synthetic(
            "\u{00a7}cServer unreachable",
            client_version,
            Some(config.max_players.cast_signed()),
        );
        let online = connection_registry.count_by_server(config_id) as i32;
        resp.players.online = online;
        resp
    }

    /// Get motd for unreachable state
    ///
    /// Return in order if defined the motd from:
    /// * Config
    /// * Stale cache
    /// * Config default
    /// * Or a default value
    fn get_unreachable_motd(
        &self,
        config: &ServerConfig,
        connection_registry: &ConnectionRegistry,
        config_id: &str,
        client_version: ProtocolVersion,
        relay_just_failed: bool,
    ) -> Backed {
        if let Some(motd) = &config.motd.unreachable {
            Backed::Synthetic(ServerPingResponse::from_entry(
                motd,
                client_version,
                motd.max_players.map(u32::cast_signed),
            ))
        } else if let Some((response, _latency)) = self.cache.get_stale(config_id) {
            if relay_just_failed {
                tracing::warn!(
                    server = config_id,
                    "serving stale cached status (backend unreachable)"
                );
            }
            Backed::Cached(response)
        } else {
            Backed::Synthetic(self.build_unreachable_motd(
                config,
                connection_registry,
                config_id,
                client_version,
            ))
        }
    }

    async fn read_status_request(
        &self,
        ctx: &mut ConnectionContext,
    ) -> Result<PacketDecoder, CoreError> {
        let mut decoder = PacketDecoder::new();
        if !ctx.buffered_data.is_empty() {
            decoder.queue_bytes(&ctx.buffered_data);
            ctx.buffered_data.clear();
        }
        read_frame_within(
            ctx.stream_mut(),
            &mut decoder,
            STATUS_READ_TIMEOUT,
            "status request",
        )
        .await?;
        Ok(decoder)
    }

    async fn handle_ping_pong(
        &self,
        ctx: &mut ConnectionContext,
        decoder: &mut PacketDecoder,
    ) -> Result<(), CoreError> {
        let frame = read_frame_within(
            ctx.stream_mut(),
            decoder,
            STATUS_READ_TIMEOUT,
            "ping request",
        )
        .await?;

        let payload = self
            .registry
            .decode_as::<SPingRequest>(&frame, STATUS_PROTOCOL_VERSION)?
            .map_or(0, |ping| ping.payload);

        let pong = CPingResponse { payload };
        send_packet(
            &self.registry,
            ctx.stream_mut(),
            &pong,
            STATUS_PROTOCOL_VERSION,
        )
        .await
    }
}
