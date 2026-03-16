use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use infrarust_config::MotdConfig;
use infrarust_protocol::legacy::{LegacyPingVariant, parse_legacy_ping};
use infrarust_protocol::{CURRENT_MC_PROTOCOL, CURRENT_MC_VERSION, LegacyPingResponse};

use infrarust_server_manager::{ServerManagerService, ServerState};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;

/// Handles legacy Minecraft ping requests (pre-1.7 clients).
///
/// Supports three variants: Beta (0xFE), 1.4 (0xFE01), and 1.6 (0xFE01FA).
pub struct LegacyHandler {
    domain_router: Arc<DomainRouter>,
    default_motd: Option<MotdConfig>,
    server_manager: Option<Arc<ServerManagerService>>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl LegacyHandler {
    /// Creates a new legacy handler with shared config state.
    pub fn new(
        domain_router: Arc<DomainRouter>,
        default_motd: Option<MotdConfig>,
        server_manager: Option<Arc<ServerManagerService>>,
        connection_registry: Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            domain_router,
            default_motd,
            server_manager,
            connection_registry,
        }
    }

    /// Handles a legacy ping connection.
    pub async fn handle(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
        // Read remaining data after the initial 0xFE byte
        let mut data = Vec::new();
        // The first byte (0xFE) is in buffered_data, skip it
        if ctx.buffered_data.len() > 1 {
            data.extend_from_slice(&ctx.buffered_data[1..]);
        }

        // Try reading more data with a short timeout
        let mut buf = [0u8; 512];
        match tokio::time::timeout(
            std::time::Duration::from_millis(100),
            ctx.stream_mut().read(&mut buf),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => data.extend_from_slice(&buf[..n]),
            _ => {} // Timeout or closed — we work with what we have
        }

        let request = parse_legacy_ping(&data)?;

        // Resolve MOTD and player counts based on variant
        let (motd, online_players, max_players) = match request.hostname.as_ref() {
            Some(hostname) => self.resolve_with_hostname(hostname),
            None => self.resolve_without_hostname(),
        };

        let response = LegacyPingResponse {
            protocol_version: CURRENT_MC_PROTOCOL,
            server_version: CURRENT_MC_VERSION.to_string(),
            motd,
            online_players,
            max_players,
        };

        let response_bytes = match request.variant {
            LegacyPingVariant::Beta => response.build_beta_response()?,
            LegacyPingVariant::V1_4 | LegacyPingVariant::V1_6 => response.build_v1_4_response()?,
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

    /// Resolves MOTD for 1.6 pings (hostname available via MC|PingHost).
    fn resolve_with_hostname(&self, hostname: &str) -> (String, i32, i32) {
        let domain = hostname.to_lowercase();

        let Some((_provider_id, cfg)) = self.domain_router.resolve(&domain) else {
            return self.resolve_from_default_motd();
        };

        let config_id = cfg.effective_id();

        // Check server manager state
        if cfg.server_manager.is_some()
            && let Some(ref sm) = self.server_manager
            && let Some(state) = sm.get_state(&config_id)
            && state != ServerState::Online
        {
            return self.resolve_state_motd(&cfg, state, &config_id);
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
            .unwrap_or(cfg.max_players) as i32;

        (motd, online, max)
    }

    /// Resolves MOTD for Beta/1.4 pings (no hostname).
    fn resolve_without_hostname(&self) -> (String, i32, i32) {
        self.resolve_from_default_motd()
    }

    /// Resolves MOTD from `default_motd` config.
    fn resolve_from_default_motd(&self) -> (String, i32, i32) {
        let entry = self.default_motd.as_ref().and_then(|m| m.online.as_ref());

        let motd = entry.map_or_else(|| "An Infrarust Proxy".to_string(), |e| e.text.clone());
        let online = self.connection_registry.count() as i32;
        let max = entry.and_then(|e| e.max_players).unwrap_or(0) as i32;

        (motd, online, max)
    }

    /// Resolves MOTD for non-online server states (sleeping, starting, etc.).
    fn resolve_state_motd(
        &self,
        cfg: &infrarust_config::ServerConfig,
        state: ServerState,
        config_id: &str,
    ) -> (String, i32, i32) {
        let (motd_entry, default_text) = match state {
            ServerState::Sleeping => (
                cfg.motd.sleeping.as_ref(),
                "\u{00a7}7Server sleeping \u{2014} \u{00a7}aConnect to wake up!",
            ),
            ServerState::Starting => (cfg.motd.starting.as_ref(), "\u{00a7}eServer is starting..."),
            ServerState::Crashed => (cfg.motd.crashed.as_ref(), "\u{00a7}cServer unavailable"),
            ServerState::Stopping => (cfg.motd.stopping.as_ref(), "\u{00a7}6Server is stopping..."),
            _ => (None, "A Minecraft Server"),
        };

        let motd = motd_entry.map_or_else(|| default_text.to_string(), |e| e.text.clone());
        let online = self.connection_registry.count_by_server(config_id) as i32;
        let max = motd_entry
            .and_then(|e| e.max_players)
            .unwrap_or(cfg.max_players) as i32;

        (motd, online, max)
    }

    /// Returns the default MOTD text from config or the hardcoded fallback.
    fn default_motd_text(&self) -> String {
        self.default_motd
            .as_ref()
            .and_then(|m| m.online.as_ref())
            .map_or_else(|| "An Infrarust Proxy".to_string(), |e| e.text.clone())
    }
}
