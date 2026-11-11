use tokio::io::{AsyncReadExt, AsyncWriteExt};

use infrarust_api::events::proxy::{PingResponse, ProxyPingEvent};
use infrarust_api::types::{Component, LEGACY_SECTION, ServerId};
use infrarust_config::{ServerAddress, ServerConfig};
use infrarust_protocol::legacy::{LegacyPingRequest, LegacyPingVariant};
use infrarust_protocol::{LegacyPingResponse, ProtocolVersion};
use infrarust_server_manager::ServerState;

use super::LegacyHandler;
use crate::error::CoreError;
use crate::loadbalancer::peek_backend_addresses;
use crate::pipeline::admission;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::RoutingData;
use crate::status::motd::{DEFAULT_PROXY_MOTD, default_entry, state_max_players, state_motd};
use crate::status::relay_gate::RelayTurn;

const LEGACY_REPLY_PREFIX: &str = "\u{a7}1\0";

impl LegacyHandler {
    pub(crate) async fn handle_ping(
        &self,
        ctx: &mut ConnectionContext,
        request: &LegacyPingRequest,
        raw: &[u8],
    ) -> Result<(), CoreError> {
        let virtual_host = admission::virtual_host(ctx);
        let routing = ctx.extensions.get::<RoutingData>().cloned();
        let (server, response) = match routing {
            Some(routing) => (
                Some(ServerId::new(routing.server_config.effective_id())),
                self.backend_response(ctx, &routing, request.variant, raw)
                    .await,
            ),
            None => (None, self.config_response(None)),
        };

        let response = self
            .fire_ping(ctx, server, virtual_host, request, response)
            .await;
        let bytes = match request.variant {
            LegacyPingVariant::Beta => response.build_beta_response()?,
            LegacyPingVariant::V1_4 | LegacyPingVariant::V1_6 => response.build_v1_4_response()?,
        };
        ctx.stream_mut().write_all(&bytes).await?;
        ctx.stream_mut().flush().await?;

        tracing::debug!(
            variant = ?request.variant,
            hostname = ?request.hostname,
            "legacy ping handled"
        );

        Ok(())
    }

    async fn backend_response(
        &self,
        ctx: &ConnectionContext,
        routing: &RoutingData,
        variant: LegacyPingVariant,
        raw: &[u8],
    ) -> LegacyPingResponse {
        let config = &routing.server_config;
        let config_id = routing.config_id.as_str();
        if let Some(cached) = self.status_cache.get_fresh_legacy(config_id, variant) {
            return cached;
        }
        let leader = match self.relay_gate.enter(config_id).await {
            RelayTurn::Leader(leader) => leader,
            RelayTurn::Unreachable | RelayTurn::Busy => return self.config_response(Some(config)),
        };
        if let Some(cached) = self.status_cache.get_fresh_legacy(config_id, variant) {
            return cached;
        }

        let addresses = peek_backend_addresses(
            config,
            routing.load_balancer.as_ref(),
            self.services.backend_load.as_ref(),
            self.services.backend_health.as_ref(),
        )
        .to_vec();
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.forward_ping_to_backend(raw, config, &addresses, ctx),
        )
        .await
        {
            Ok(Ok(response)) => {
                self.status_cache
                    .put_legacy(config_id, variant, response.clone());
                leader.succeeded();
                response
            }
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "ping passthrough failed, using fallback");
                leader.failed();
                self.config_response(Some(config))
            }
            Err(_) => {
                tracing::debug!("ping passthrough timed out, using fallback");
                leader.failed();
                self.config_response(Some(config))
            }
        }
    }

    async fn fire_ping(
        &self,
        ctx: &ConnectionContext,
        server: Option<ServerId>,
        virtual_host: Option<String>,
        request: &LegacyPingRequest,
        mut response: LegacyPingResponse,
    ) -> LegacyPingResponse {
        let sent = PingResponse::new(
            Component::text(response.motd.clone()),
            response.max_players,
            response.online_players,
            infrarust_api::types::ProtocolVersion::new(response.protocol_version),
            response.server_version.clone(),
            None,
        );
        let event = ProxyPingEvent::new(
            ctx.client_addr(),
            server,
            virtual_host,
            infrarust_api::types::ProtocolVersion::new(
                request.protocol_version.map_or(0, i32::from),
            ),
            true,
            sent.clone(),
        );
        let event = self.services.event_bus.fire(event).await;
        let answered = &event.response;
        if answered.description != sent.description {
            response.motd = answered.description.to_legacy(LEGACY_SECTION);
        }
        response.max_players = answered.max_players;
        response.online_players = answered.online_players;
        response.protocol_version = answered.protocol_version.raw();
        response.server_version.clone_from(&answered.version_name);
        response
    }

    async fn forward_ping_to_backend(
        &self,
        raw_ping: &[u8],
        config: &ServerConfig,
        addresses: &[ServerAddress],
        ctx: &ConnectionContext,
    ) -> Result<LegacyPingResponse, CoreError> {
        let config_id = config.effective_id();

        let mut backend = self
            .backend_connector
            .connect(
                &config_id,
                addresses,
                config.timeouts.as_ref().map(|t| t.connect),
                false,
                &ctx.connection_info(),
            )
            .await?;

        backend.stream_mut().write_all(raw_ping).await?;
        backend.stream_mut().flush().await?;

        let reply = read_legacy_kick_text(backend.stream_mut()).await?;
        parse_legacy_reply(&reply)
    }

    fn config_response(&self, config: Option<&ServerConfig>) -> LegacyPingResponse {
        let registry = &self.services.connection_registry;
        let (motd, online, max) = if let Some(cfg) = config {
            let config_id = cfg.effective_id();

            if cfg.server_manager.is_some()
                && let Some(ref sm) = self.services.server_manager
                && let Some(state) = sm.get_state(&config_id)
                && state != ServerState::Online
            {
                return self.state_response(cfg, state, &config_id);
            }

            let motd = cfg
                .motd
                .online
                .as_ref()
                .map_or_else(|| self.default_motd_text(), |m| m.text.clone());
            let online = registry.count_by_server(&config_id) as i32;
            let max = cfg
                .motd
                .online
                .as_ref()
                .and_then(|m| m.max_players)
                .unwrap_or(cfg.max_players)
                .cast_signed();
            (motd, online, max)
        } else {
            let entry = default_entry(&self.services.config);
            let motd = entry.map_or_else(|| DEFAULT_PROXY_MOTD.to_string(), |e| e.text.clone());
            let online = registry.count() as i32;
            let max = entry.and_then(|e| e.max_players).unwrap_or(0).cast_signed();
            (motd, online, max)
        };

        LegacyPingResponse {
            protocol_version: ProtocolVersion::CURRENT.0,
            server_version: ProtocolVersion::CURRENT.name().to_string(),
            motd,
            online_players: online,
            max_players: max,
        }
    }

    fn state_response(
        &self,
        cfg: &ServerConfig,
        state: ServerState,
        config_id: &str,
    ) -> LegacyPingResponse {
        let (entry, default_text) = state_motd(&cfg.motd, state);

        LegacyPingResponse {
            protocol_version: ProtocolVersion::CURRENT.0,
            server_version: ProtocolVersion::CURRENT.name().to_string(),
            motd: entry.map_or_else(|| default_text.to_string(), |e| e.text.clone()),
            online_players: self.services.connection_registry.count_by_server(config_id) as i32,
            max_players: state_max_players(entry, cfg),
        }
    }

    fn default_motd_text(&self) -> String {
        default_entry(&self.services.config)
            .map_or_else(|| DEFAULT_PROXY_MOTD.to_string(), |e| e.text.clone())
    }
}

async fn read_legacy_kick_text(stream: &mut tokio::net::TcpStream) -> Result<String, CoreError> {
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
    let str_len = usize::from(u16::from_be_bytes(len_bytes));
    if str_len > 32767 {
        return Err(CoreError::Protocol(
            infrarust_protocol::ProtocolError::invalid(format!(
                "legacy kick string length too large: {str_len}"
            )),
        ));
    }

    let mut payload = vec![0u8; str_len * 2];
    stream.read_exact(&mut payload).await?;
    let units: Vec<u16> = payload
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair))
        .collect();
    String::from_utf16(&units).map_err(|_| {
        CoreError::Protocol(infrarust_protocol::ProtocolError::invalid(
            "legacy kick is not UTF-16",
        ))
    })
}

fn parse_legacy_reply(reply: &str) -> Result<LegacyPingResponse, CoreError> {
    let invalid = || {
        CoreError::Protocol(infrarust_protocol::ProtocolError::invalid(format!(
            "unreadable legacy ping reply {reply:?}"
        )))
    };
    let number = |value: &str| value.parse::<i32>().map_err(|_| invalid());
    if let Some(fields) = reply.strip_prefix(LEGACY_REPLY_PREFIX) {
        let parts: Vec<&str> = fields.split('\0').collect();
        let [protocol, version, motd, online, max] = parts[..] else {
            return Err(invalid());
        };
        return Ok(LegacyPingResponse {
            protocol_version: number(protocol)?,
            server_version: version.to_string(),
            motd: motd.to_string(),
            online_players: number(online)?,
            max_players: number(max)?,
        });
    }
    let mut parts = reply.rsplitn(3, LEGACY_SECTION);
    let (Some(max), Some(online), Some(motd)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid());
    };
    Ok(LegacyPingResponse {
        protocol_version: 0,
        server_version: String::new(),
        motd: motd.to_string(),
        online_players: number(online)?,
        max_players: number(max)?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;

    use infrarust_transport::BackendConnector;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::status::StatusCache;

    #[test]
    fn legacy_and_modern_state_motds_report_the_same_max_players_and_text() {
        let services = test_proxy_services();
        let handler = LegacyHandler::new(
            services,
            Arc::new(BackendConnector::new(
                std::time::Duration::from_secs(2),
                infrarust_config::KeepaliveConfig::default(),
            )),
            CancellationToken::new(),
            Arc::new(StatusCache::new(std::time::Duration::from_secs(5))),
        );
        let mut cfg: ServerConfig = toml::from_str(
            "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"127.0.0.1:25566\"]\nmax_players = 42\n",
        )
        .unwrap();

        for state in [
            ServerState::Sleeping,
            ServerState::Starting,
            ServerState::Crashed,
            ServerState::Stopping,
        ] {
            let legacy = handler.state_response(&cfg, state, "lobby");
            let modern = crate::status::StatusHandler::build_state_motd(
                &cfg,
                state,
                ProtocolVersion::CURRENT,
            );
            assert_eq!(modern.players.max, legacy.max_players, "{state:?}");
            assert_eq!(modern.players.max, 42, "{state:?}");
            assert_eq!(modern.description["text"], legacy.motd, "{state:?}");
        }

        cfg.motd.sleeping = Some(infrarust_config::MotdEntry {
            text: "zzz".into(),
            favicon: None,
            version_name: None,
            version_protocol: None,
            max_players: Some(7),
        });
        let legacy = handler.state_response(&cfg, ServerState::Sleeping, "lobby");
        let modern = crate::status::StatusHandler::build_state_motd(
            &cfg,
            ServerState::Sleeping,
            ProtocolVersion::CURRENT,
        );
        assert_eq!((modern.players.max, legacy.max_players), (7, 7));
        assert_eq!(modern.description["text"], "zzz");
        assert_eq!(legacy.motd, "zzz");
    }

    #[test]
    fn legacy_replies_are_read_in_both_formats() {
        let modern =
            parse_legacy_reply("\u{a7}1\u{0}78\u{0}1.6.4\u{0}\u{a7}6Hello\u{0}3\u{0}20").unwrap();
        assert_eq!(modern.protocol_version, 78);
        assert_eq!(modern.server_version, "1.6.4");
        assert_eq!(modern.motd, "\u{a7}6Hello");
        assert_eq!((modern.online_players, modern.max_players), (3, 20));

        let beta = parse_legacy_reply("A \u{a7}cred server\u{a7}4\u{a7}10").unwrap();
        assert_eq!(beta.motd, "A \u{a7}cred server");
        assert_eq!((beta.online_players, beta.max_players), (4, 10));

        assert!(parse_legacy_reply("\u{a7}1\u{0}78\u{0}1.6.4").is_err());
        assert!(parse_legacy_reply("no separators").is_err());
    }
}
