use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use infrarust_config::{DomainIndex, ServerConfig};
use infrarust_protocol::legacy::{LegacyPingVariant, parse_legacy_ping};
use infrarust_protocol::{CURRENT_MC_PROTOCOL, CURRENT_MC_VERSION, LegacyPingResponse};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;

/// Handles legacy Minecraft ping requests (pre-1.7 clients).
///
/// Supports three variants: Beta (0xFE), 1.4 (0xFE01), and 1.6 (0xFE01FA).
pub struct LegacyHandler {
    domain_index: Arc<ArcSwap<DomainIndex>>,
    configs: Arc<ArcSwap<HashMap<String, Arc<ServerConfig>>>>,
}

impl LegacyHandler {
    /// Creates a new legacy handler with shared config state.
    pub fn new(
        domain_index: Arc<ArcSwap<DomainIndex>>,
        configs: Arc<ArcSwap<HashMap<String, Arc<ServerConfig>>>>,
    ) -> Self {
        Self {
            domain_index,
            configs,
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

        // Try to resolve domain for 1.6 variant
        let motd = if let Some(ref hostname) = request.hostname {
            let domain = hostname.to_lowercase();
            let index = self.domain_index.load();
            if let Some(config_id) = index.resolve(&domain) {
                let configs = self.configs.load();
                configs
                    .get(config_id)
                    .and_then(|cfg| cfg.motd.online.as_ref().map(|m| m.text.clone()))
                    .unwrap_or_else(|| "An Infrarust Proxy".to_string())
            } else {
                "An Infrarust Proxy".to_string()
            }
        } else {
            "An Infrarust Proxy".to_string()
        };

        let response = LegacyPingResponse {
            protocol_version: CURRENT_MC_PROTOCOL,
            server_version: CURRENT_MC_VERSION.to_string(),
            motd,
            online_players: 0,
            max_players: 0,
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
}
