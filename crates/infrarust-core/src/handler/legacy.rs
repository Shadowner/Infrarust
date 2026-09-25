use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use infrarust_api::events::proxy::{PingResponse, ProxyPingEvent};
use infrarust_api::types::{Component, LEGACY_SECTION, ServerId};
use infrarust_config::{ServerAddress, ServerConfig};
use infrarust_protocol::legacy::{
    LegacyPingRequest, LegacyPingVariant, parse_legacy_handshake, parse_legacy_ping,
};
use infrarust_protocol::{CURRENT_MC_PROTOCOL, CURRENT_MC_VERSION, LegacyPingResponse};

use infrarust_server_manager::ServerState;
use infrarust_transport::BackendConnector;

use super::forwarded::{Arrival, ForwardedLogin, Opening, Route, UNKNOWN_SERVER, Wire};
use super::helpers::send_legacy_kick;
use crate::error::CoreError;
use crate::loadbalancer::{peek_backend_addresses, select_backend_addresses};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::RoutingData;
use crate::services::ProxyServices;
use crate::util::normalize_handshake;

const DEFAULT_MOTD: &str = "An Infrarust Proxy";
const LEGACY_REPLY_PREFIX: &str = "\u{a7}1\0";

pub struct LegacyHandler {
    services: ProxyServices,
    backend_connector: Arc<BackendConnector>,
    shutdown: CancellationToken,
}

impl LegacyHandler {
    pub fn new(
        services: ProxyServices,
        backend_connector: Arc<BackendConnector>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            services,
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

        let virtual_host = request
            .hostname
            .as_deref()
            .map(|host| normalize_handshake(host).to_lowercase());
        let route = virtual_host
            .as_deref()
            .and_then(|host| self.services.domain_router.resolve_route(host));

        let (server, response) = match route {
            Some((_provider_id, config, load_balancer)) => {
                let addresses = peek_backend_addresses(
                    &config,
                    load_balancer.as_ref(),
                    self.services.backend_load.as_ref(),
                    self.services.backend_health.as_ref(),
                )
                .to_vec();
                let full_ping = reconstruct_ping_packet(&raw_data);
                let response = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.forward_ping_to_backend(&full_ping, &config, &addresses, ctx),
                )
                .await
                {
                    Ok(Ok(response)) => response,
                    Ok(Err(e)) => {
                        tracing::debug!(error = %e, "ping passthrough failed, using fallback");
                        self.config_response(Some(&config))
                    }
                    Err(_) => {
                        tracing::debug!("ping passthrough timed out, using fallback");
                        self.config_response(Some(&config))
                    }
                };
                (Some(ServerId::new(config.effective_id())), response)
            }
            None => (None, self.config_response(None)),
        };

        let response = self
            .fire_ping(ctx, server, virtual_host, &request, response)
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
            let entry = self
                .services
                .config
                .default_motd
                .as_ref()
                .and_then(|m| m.online.as_ref());
            let motd = entry.map_or_else(|| DEFAULT_MOTD.to_string(), |e| e.text.clone());
            let online = registry.count() as i32;
            let max = entry.and_then(|e| e.max_players).unwrap_or(0).cast_signed();
            (motd, online, max)
        };

        LegacyPingResponse {
            protocol_version: CURRENT_MC_PROTOCOL,
            server_version: CURRENT_MC_VERSION.to_string(),
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

        LegacyPingResponse {
            protocol_version: CURRENT_MC_PROTOCOL,
            server_version: CURRENT_MC_VERSION.to_string(),
            motd: motd_entry.map_or_else(|| default_text.to_string(), |e| e.text.clone()),
            online_players: self.services.connection_registry.count_by_server(config_id) as i32,
            max_players: motd_entry
                .and_then(|e| e.max_players)
                .unwrap_or(cfg.max_players)
                .cast_signed(),
        }
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
            if next[0] == 0x01
                && let Ok(Ok(more)) = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    self.read_remaining_v1_6_data(ctx),
                )
                .await
            {
                data.extend_from_slice(&more);
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

        let handshake = parse_legacy_handshake(&raw_data[1..])?;

        tracing::debug!(
            protocol = handshake.protocol_version,
            username = %handshake.username,
            hostname = %handshake.hostname,
            port = handshake.port,
            "legacy login handshake"
        );

        let domain = normalize_handshake(&handshake.hostname).to_lowercase();
        let Some((_provider_id, server_config, load_balancer)) =
            self.services.domain_router.resolve_route(&domain)
        else {
            tracing::debug!(domain = %domain, "legacy login: unknown domain");
            send_legacy_kick(ctx.stream_mut(), &Component::text(UNKNOWN_SERVER))
                .await
                .ok();
            return Ok(());
        };

        if let Some(ban) = self
            .services
            .ban_manager
            .check_player(&ctx.client_ip, &handshake.username, None)
            .await?
        {
            tracing::info!(
                ip = %ctx.client_ip,
                username = %handshake.username,
                ban_type = ban.target.display_type(),
                "legacy connection rejected: player is banned"
            );
            send_legacy_kick(ctx.stream_mut(), &Component::text(ban.kick_message()))
                .await
                .ok();
            return Ok(());
        }

        let addresses = select_backend_addresses(
            &server_config,
            load_balancer.as_ref(),
            self.services.pending_backends.as_ref(),
            self.services.backend_health.as_ref(),
        )
        .to_vec();
        if let Some(first) = addresses.first() {
            ctx.extensions
                .insert(self.services.pending_backends.reserve(first));
        }
        let origin = Route {
            routing: RoutingData {
                config_id: server_config.effective_id(),
                server_config,
                load_balancer,
            },
            addresses,
        };
        let arrival = Arrival {
            username: handshake.username.clone(),
            claimed_uuid: None,
            protocol_version: infrarust_api::types::ProtocolVersion::new(i32::from(
                handshake.protocol_version,
            )),
            domain,
        };
        let login = ForwardedLogin {
            services: &self.services,
            connector: &self.backend_connector,
            shutdown: &self.shutdown,
            wire: Wire::Legacy,
        };
        let Some(ready) = login
            .open(ctx, arrival, origin, Opening::Legacy(&raw_data))
            .await?
        else {
            return Ok(());
        };

        let server = ready.server.clone();
        tracing::info!(
            server = %server,
            username = %handshake.username,
            "legacy login: forwarding to backend"
        );

        let result = ready.forward(ctx.take_stream()).await;

        tracing::info!(
            server = %server,
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

    fn default_motd_text(&self) -> String {
        self.services
            .config
            .default_motd
            .as_ref()
            .and_then(|m| m.online.as_ref())
            .map_or_else(|| DEFAULT_MOTD.to_string(), |e| e.text.clone())
    }
}

fn reconstruct_ping_packet(raw_data: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(1 + raw_data.len());
    packet.push(0xFE);
    packet.extend_from_slice(raw_data);
    packet
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
    use std::net::{IpAddr, Ipv4Addr};

    use tokio::net::{TcpListener, TcpStream};

    use super::*;
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::loadbalancer::AddressConnectionCount;
    use crate::provider::ProviderId;

    fn utf16be(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut out = u16::try_from(units.len()).unwrap().to_be_bytes().to_vec();
        for unit in units {
            out.extend_from_slice(&unit.to_be_bytes());
        }
        out
    }

    fn legacy_login_tail(username: &str, hostname: &str, port: u16) -> Vec<u8> {
        let mut out = vec![0x3C];
        out.extend_from_slice(&utf16be(username));
        out.extend_from_slice(&utf16be(hostname));
        out.extend_from_slice(&i32::from(port).to_be_bytes());
        out
    }

    async fn test_ctx(stream: TcpStream) -> ConnectionContext {
        let peer = stream.local_addr().unwrap();
        let local = stream.peer_addr().unwrap();
        let mut ctx =
            ConnectionContext::new_for_test(stream, peer, IpAddr::V4(Ipv4Addr::LOCALHOST), local);
        ctx.buffered_data.extend_from_slice(&[0x02]);
        ctx
    }

    #[tokio::test]
    async fn a_legacy_login_is_counted_on_its_backend_for_the_whole_forward() {
        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_addr = backend_listener.local_addr().unwrap();
        let address: ServerAddress = backend_addr.to_string().parse().unwrap();

        let services = test_proxy_services();
        services.domain_router.add(
            ProviderId::file("lobby"),
            toml::from_str(&format!(
                "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"{backend_addr}\"]\n"
            ))
            .unwrap(),
        );
        let load = Arc::clone(&services.backend_load);
        let handler = Arc::new(LegacyHandler::new(
            services,
            Arc::new(BackendConnector::new(
                std::time::Duration::from_secs(2),
                infrarust_config::KeepaliveConfig::default(),
            )),
            CancellationToken::new(),
        ));

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let (server_side, _) = proxy_listener.accept().await.unwrap();

        client
            .write_all(&legacy_login_tail("Notch", "lobby.test", 25565))
            .await
            .unwrap();
        client.flush().await.unwrap();

        let mut ctx = test_ctx(server_side).await;
        let login = {
            let handler = Arc::clone(&handler);
            tokio::spawn(async move { handler.handle_login(&mut ctx).await })
        };

        let (mut backend, _) = backend_listener.accept().await.unwrap();
        let mut forwarded = [0u8; 1];
        backend.read_exact(&mut forwarded).await.unwrap();
        assert_eq!(forwarded[0], 0x02);
        assert_eq!(
            load.active_connections_for_address(&address),
            1,
            "a forwarded legacy session must be visible to least_conn"
        );

        drop(client);
        drop(backend);
        login.await.unwrap().unwrap();

        assert_eq!(
            load.active_connections_for_address(&address),
            0,
            "the count must be given back when the forward ends"
        );
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
