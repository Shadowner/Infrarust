use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use infrarust_config::MotdConfig;
use infrarust_protocol::legacy::{
    LegacyPingVariant, build_legacy_kick, parse_legacy_handshake, parse_legacy_ping,
};
use infrarust_protocol::{CURRENT_MC_PROTOCOL, CURRENT_MC_VERSION, LegacyPingResponse};

use infrarust_server_manager::{ServerManagerService, ServerState};
use infrarust_transport::{BackendConnector, select_forwarder};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;

/// Handles legacy Minecraft connections (pre-1.7 clients).
///
/// Supports:
/// - Legacy ping: Beta (0xFE), 1.4 (0xFE01), and 1.6 (0xFE01FA)
/// - Legacy login: 0x02 handshake with passthrough proxying
pub struct LegacyHandler {
    domain_router: Arc<DomainRouter>,
    default_motd: Option<MotdConfig>,
    server_manager: Option<Arc<ServerManagerService>>,
    connection_registry: Arc<ConnectionRegistry>,
    backend_connector: Arc<BackendConnector>,
    shutdown: CancellationToken,
}

impl LegacyHandler {
    pub fn new(
        domain_router: Arc<DomainRouter>,
        default_motd: Option<MotdConfig>,
        server_manager: Option<Arc<ServerManagerService>>,
        connection_registry: Arc<ConnectionRegistry>,
        backend_connector: Arc<BackendConnector>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            domain_router,
            default_motd,
            server_manager,
            connection_registry,
            backend_connector,
            shutdown,
        }
    }

    /// Handles a legacy connection (ping or login).
    /// Dispatches based on the first byte: `0xFE` → ping, `0x02` → login.
    ///
    /// # Errors
    /// Returns `CoreError` on I/O or protocol errors.
    pub async fn handle(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
        let first_byte = ctx.buffered_data.first().copied().unwrap_or(0);

        match first_byte {
            0xFE => self.handle_ping(ctx).await,
            0x02 => self.handle_login(ctx).await,
            _ => {
                tracing::debug!(byte = first_byte, "unknown legacy first byte");
                Ok(())
            }
        }
    }

    async fn handle_ping(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
        let raw_data = self.read_legacy_ping_data(ctx).await?;

        let request = parse_legacy_ping(&raw_data)?;

        tracing::debug!(
            variant = ?request.variant,
            hostname = ?request.hostname,
            "legacy ping parsed"
        );

        // For Beta/V1_4 (no hostname), send a generic response
        let hostname = match &request.hostname {
            Some(h) => h.clone(),
            None => {
                let response = self.build_config_response(&request.variant, None);
                ctx.stream_mut().write_all(&response).await?;
                ctx.stream_mut().flush().await?;
                return Ok(());
            }
        };

        let server_config = self.domain_router.resolve(&hostname.to_lowercase());

        let response_bytes = match server_config {
            Some((_provider_id, config)) => {
                let full_ping = self.reconstruct_ping_packet(&raw_data);
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.forward_ping_to_backend(&full_ping, &config, ctx),
                )
                .await
                {
                    Ok(Ok(bytes)) => bytes,
                    Ok(Err(e)) => {
                        tracing::debug!(error = %e, "ping passthrough failed, using fallback");
                        self.build_config_response(&request.variant, Some(&config))
                    }
                    Err(_) => {
                        tracing::debug!("ping passthrough timed out, using fallback");
                        self.build_config_response(&request.variant, Some(&config))
                    }
                }
            }
            None => self.build_config_response(&request.variant, None),
        };

        ctx.stream_mut().write_all(&response_bytes).await?;
        ctx.stream_mut().flush().await?;

        tracing::debug!(
            variant = ?request.variant,
            hostname = ?request.hostname,
            "legacy ping handled"
        );

        Ok(())
    }

    async fn forward_ping_to_backend(
        &self,
        raw_ping: &[u8],
        config: &infrarust_config::ServerConfig,
        ctx: &ConnectionContext,
    ) -> Result<Vec<u8>, CoreError> {
        let config_id = config.effective_id();

        let mut backend = self
            .backend_connector
            .connect(
                &config_id,
                &config.addresses,
                config.timeouts.as_ref().map(|t| t.connect),
                false, // No proxy protocol for legacy pings
                &ctx.connection_info(),
            )
            .await?;

        backend.stream_mut().write_all(raw_ping).await?;
        backend.stream_mut().flush().await?;

        let response = Self::read_legacy_kick_response(backend.stream_mut()).await?;

        Ok(response)
    }

    async fn read_legacy_kick_response(
        stream: &mut tokio::net::TcpStream,
    ) -> Result<Vec<u8>, CoreError> {
        let mut packet_id = [0u8; 1];
        stream.read_exact(&mut packet_id).await?;

        if packet_id[0] != 0xFF {
            return Err(CoreError::Protocol(
                infrarust_protocol::ProtocolError::invalid(format!(
                    "expected legacy kick 0xFF, got 0x{:02X}",
                    packet_id[0]
                )),
            ));
        }

        let mut len_bytes = [0u8; 2];
        stream.read_exact(&mut len_bytes).await?;
        let str_len = u16::from_be_bytes(len_bytes) as usize;

        if str_len > 32767 {
            return Err(CoreError::Protocol(
                infrarust_protocol::ProtocolError::invalid(format!(
                    "legacy kick string length too large: {str_len}"
                )),
            ));
        }

        let byte_len = str_len * 2;
        let mut payload = vec![0u8; byte_len];
        stream.read_exact(&mut payload).await?;

        let mut result = Vec::with_capacity(1 + 2 + byte_len);
        result.push(0xFF);
        result.extend_from_slice(&len_bytes);
        result.extend_from_slice(&payload);

        Ok(result)
    }

    fn reconstruct_ping_packet(&self, raw_data: &[u8]) -> Vec<u8> {
        let mut packet = Vec::with_capacity(1 + raw_data.len());
        packet.push(0xFE);
        packet.extend_from_slice(raw_data);
        packet
    }

    fn build_config_response(
        &self,
        variant: &LegacyPingVariant,
        config: Option<&infrarust_config::ServerConfig>,
    ) -> Vec<u8> {
        let (motd, online, max) = if let Some(cfg) = config {
            let config_id = cfg.effective_id();

            // Check server state
            if cfg.server_manager.is_some()
                && let Some(ref sm) = self.server_manager
                && let Some(state) = sm.get_state(&config_id)
                && state != ServerState::Online
            {
                return self
                    .build_state_response(variant, cfg, state, &config_id)
                    .unwrap_or_default();
            }

            let motd = cfg
                .motd
                .online
                .as_ref()
                .map_or_else(|| self.default_motd_text(), |m| m.text.clone());
            let online = self.connection_registry.count_by_server(&config_id) as i32;
            let max = cfg
                .motd
                .online
                .as_ref()
                .and_then(|m| m.max_players)
                .unwrap_or(cfg.max_players)
                .cast_signed();
            (motd, online, max)
        } else {
            let entry = self.default_motd.as_ref().and_then(|m| m.online.as_ref());
            let motd =
                entry.map_or_else(|| "An Infrarust Proxy".to_string(), |e| e.text.clone());
            let online = self.connection_registry.count() as i32;
            let max = entry.and_then(|e| e.max_players).unwrap_or(0).cast_signed();
            (motd, online, max)
        };

        let response = LegacyPingResponse {
            protocol_version: CURRENT_MC_PROTOCOL,
            server_version: CURRENT_MC_VERSION.to_string(),
            motd,
            online_players: online,
            max_players: max,
        };

        let result = match variant {
            LegacyPingVariant::Beta => response.build_beta_response(),
            LegacyPingVariant::V1_4 | LegacyPingVariant::V1_6 => response.build_v1_4_response(),
        };

        result.unwrap_or_default()
    }

    /// Builds a state-specific ping response (sleeping, starting, etc.).
    fn build_state_response(
        &self,
        variant: &LegacyPingVariant,
        cfg: &infrarust_config::ServerConfig,
        state: ServerState,
        config_id: &str,
    ) -> Result<Vec<u8>, CoreError> {
        let (motd_entry, default_text) = match state {
            ServerState::Sleeping => (
                cfg.motd.sleeping.as_ref(),
                "\u{00a7}7Server sleeping \u{2014} \u{00a7}aConnect to wake up!",
            ),
            ServerState::Starting => {
                (cfg.motd.starting.as_ref(), "\u{00a7}eServer is starting...")
            }
            ServerState::Crashed => (cfg.motd.crashed.as_ref(), "\u{00a7}cServer unavailable"),
            ServerState::Stopping => {
                (cfg.motd.stopping.as_ref(), "\u{00a7}6Server is stopping...")
            }
            _ => (None, "A Minecraft Server"),
        };

        let motd = motd_entry.map_or_else(|| default_text.to_string(), |e| e.text.clone());
        let online = self.connection_registry.count_by_server(config_id) as i32;
        let max = motd_entry
            .and_then(|e| e.max_players)
            .unwrap_or(cfg.max_players)
            .cast_signed();

        let response = LegacyPingResponse {
            protocol_version: CURRENT_MC_PROTOCOL,
            server_version: CURRENT_MC_VERSION.to_string(),
            motd,
            online_players: online,
            max_players: max,
        };

        let bytes = match variant {
            LegacyPingVariant::Beta => response.build_beta_response()?,
            LegacyPingVariant::V1_4 | LegacyPingVariant::V1_6 => response.build_v1_4_response()?,
        };

        Ok(bytes)
    }

    /// Returns data AFTER the `0xFE` byte (which is in `buffered_data[0]`).
    async fn read_legacy_ping_data(
        &self,
        ctx: &mut ConnectionContext,
    ) -> Result<Vec<u8>, CoreError> {
        let mut data = Vec::with_capacity(128);

        if ctx.buffered_data.len() > 1 {
            data.extend_from_slice(&ctx.buffered_data[1..]);
        }

        // Beta sends nothing after 0xFE, so we need a timeout
        let mut next = [0u8; 1];
        if let Ok(Ok(_)) = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            ctx.stream_mut().read_exact(&mut next),
        )
        .await
        {
            data.push(next[0]);

            // If 0x01, try for V1.6 data (0xFA + MC|PingHost)
            if next[0] == 0x01 {
                if let Ok(Ok(more)) = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    self.read_remaining_v1_6_data(ctx),
                )
                .await
                {
                    data.extend_from_slice(&more);
                }
            }
        }

        Ok(data)
    }

    /// After `0xFE 0x01`, reads: `0xFA` + channel name + data length + remaining data.
    async fn read_remaining_v1_6_data(
        &self,
        ctx: &mut ConnectionContext,
    ) -> Result<Vec<u8>, CoreError> {
        let mut data = Vec::new();

        // Read the 0xFA byte
        let mut byte = [0u8; 1];
        ctx.stream_mut().read_exact(&mut byte).await?;
        data.push(byte[0]);

        if byte[0] != 0xFA {
            return Ok(data); // Not V1.6 format
        }

        // Read channel name string length (u16 BE)
        let mut len_bytes = [0u8; 2];
        ctx.stream_mut().read_exact(&mut len_bytes).await?;
        data.extend_from_slice(&len_bytes);
        let str_len = u16::from_be_bytes(len_bytes) as usize;

        // Read channel name (UTF-16BE)
        let mut str_data = vec![0u8; str_len * 2];
        ctx.stream_mut().read_exact(&mut str_data).await?;
        data.extend_from_slice(&str_data);

        // Read data length (u16 BE)
        let mut data_len_bytes = [0u8; 2];
        ctx.stream_mut().read_exact(&mut data_len_bytes).await?;
        data.extend_from_slice(&data_len_bytes);
        let data_len = u16::from_be_bytes(data_len_bytes) as usize;

        // Read remaining data (protocol version + hostname + port)
        let mut remaining = vec![0u8; data_len];
        ctx.stream_mut().read_exact(&mut remaining).await?;
        data.extend_from_slice(&remaining);

        Ok(data)
    }

    async fn handle_login(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
        let raw_data = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.read_legacy_handshake_data(ctx),
        )
        .await
        .map_err(|_| CoreError::Timeout("legacy handshake read timed out".into()))??;

        // Parse (data after 0x02)
        let handshake = parse_legacy_handshake(&raw_data[1..])?;

        tracing::debug!(
            protocol = handshake.protocol_version,
            username = %handshake.username,
            hostname = %handshake.hostname,
            port = handshake.port,
            "legacy login handshake"
        );

        // Route to backend
        let domain = handshake.hostname.to_lowercase();
        let Some((_provider_id, server_config)) = self.domain_router.resolve(&domain) else {
            tracing::debug!(domain = %domain, "legacy login: unknown domain");
            self.send_legacy_kick(ctx, "Unknown server").await;
            return Ok(());
        };

        let config_id = server_config.effective_id();

        // Connect to backend
        let backend = match self
            .backend_connector
            .connect(
                &config_id,
                &server_config.addresses,
                server_config.timeouts.as_ref().map(|t| t.connect),
                server_config.send_proxy_protocol,
                &ctx.connection_info(),
            )
            .await
        {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    server = %config_id,
                    error = %e,
                    "legacy login: backend unreachable"
                );
                let msg = server_config.effective_disconnect_message();
                self.send_legacy_kick(ctx, &msg).await;
                return Ok(());
            }
        };

        // Forward the raw handshake bytes to backend
        let mut backend = backend;
        backend.stream_mut().write_all(&raw_data).await?;
        backend.stream_mut().flush().await?;

        tracing::info!(
            server = %config_id,
            username = %handshake.username,
            "legacy login: forwarding to backend"
        );

        // Bidirectional forwarding
        let client_stream = ctx.take_stream();
        let backend_stream = backend.into_stream();
        let forwarder = select_forwarder(server_config.proxy_mode);

        let result = forwarder
            .forward(client_stream, backend_stream, self.shutdown.child_token())
            .await;

        tracing::info!(
            server = %config_id,
            username = %handshake.username,
            c2b = result.client_to_backend,
            b2c = result.backend_to_client,
            reason = ?result.reason,
            "legacy session ended"
        );

        Ok(())
    }

    /// Supports both pre-1.3 and 1.3+ formats.
    async fn read_legacy_handshake_data(
        &self,
        ctx: &mut ConnectionContext,
    ) -> Result<Vec<u8>, CoreError> {
        let mut data = Vec::with_capacity(256);

        // The 0x02 byte is in buffered_data
        data.push(0x02);

        // Read the format/protocol byte
        let mut format_byte = [0u8; 1];
        ctx.stream_mut().read_exact(&mut format_byte).await?;
        data.push(format_byte[0]);

        if format_byte[0] == 0x00 {
            // Pre-1.3: [0x00] [low_byte_of_string_len] [UTF-16BE connection string]
            let mut low_byte = [0u8; 1];
            ctx.stream_mut().read_exact(&mut low_byte).await?;
            data.push(low_byte[0]);

            let str_len = u16::from_be_bytes([0x00, low_byte[0]]) as usize;
            let mut str_data = vec![0u8; str_len * 2];
            ctx.stream_mut().read_exact(&mut str_data).await?;
            data.extend_from_slice(&str_data);
        } else {
            // 1.3+: [protocol] [string16 username] [string16 hostname] [i32 port]
            self.read_legacy_string_into(ctx, &mut data).await?;
            self.read_legacy_string_into(ctx, &mut data).await?;
            let mut port = [0u8; 4];
            ctx.stream_mut().read_exact(&mut port).await?;
            data.extend_from_slice(&port);
        }

        Ok(data)
    }

    /// Format: `u16 BE char_count` + `char_count * 2` bytes of UTF-16BE.
    async fn read_legacy_string_into(
        &self,
        ctx: &mut ConnectionContext,
        data: &mut Vec<u8>,
    ) -> Result<(), CoreError> {
        let mut len_bytes = [0u8; 2];
        ctx.stream_mut().read_exact(&mut len_bytes).await?;
        let char_count = u16::from_be_bytes(len_bytes) as usize;

        let mut str_data = vec![0u8; char_count * 2];
        ctx.stream_mut().read_exact(&mut str_data).await?;

        data.extend_from_slice(&len_bytes);
        data.extend_from_slice(&str_data);
        Ok(())
    }

    async fn send_legacy_kick(&self, ctx: &mut ConnectionContext, reason: &str) {
        if let Ok(kick_bytes) = build_legacy_kick(reason) {
            let _ = ctx.stream_mut().write_all(&kick_bytes).await;
            let _ = ctx.stream_mut().flush().await;
        }
    }

    /// Returns the default MOTD text from config or the hardcoded fallback.
    fn default_motd_text(&self) -> String {
        self.default_motd
            .as_ref()
            .and_then(|m| m.online.as_ref())
            .map_or_else(|| "An Infrarust Proxy".to_string(), |e| e.text.clone())
    }
}
