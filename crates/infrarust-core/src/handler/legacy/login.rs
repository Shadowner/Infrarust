use tokio::io::AsyncReadExt;

use infrarust_api::events::handshake::{ConnectionHandshakeEvent, HandshakeIntent, RejectReason};
use infrarust_api::services::ban_service::LoginAttempt;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::legacy::parse_legacy_handshake;

use super::LegacyHandler;
use crate::error::CoreError;
use crate::handler::forwarded::{Arrival, ForwardedLogin, Opening, Route, UNKNOWN_SERVER, Wire};
use crate::handler::helpers::send_legacy_kick;
use crate::loadbalancer::select_backend_addresses;
use crate::pipeline::admission::{self, Admission};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::RoutingData;
use crate::util::normalize_handshake;

impl LegacyHandler {
    pub(super) async fn handle_login(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
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
            admission::reject(
                &self.services.event_bus,
                ctx.client_addr(),
                Some(domain),
                RejectReason::UnknownDomain,
            );
            send_legacy_kick(ctx.stream_mut(), &Component::text(UNKNOWN_SERVER))
                .await
                .ok();
            return Ok(());
        };

        let screened = admission::screen(&self.services.event_bus, || {
            ConnectionHandshakeEvent::new(
                ctx.client_addr(),
                HandshakeIntent::Login,
                infrarust_api::types::ProtocolVersion::new(i32::from(handshake.protocol_version)),
            )
            .with_host(
                handshake.hostname.clone(),
                Some(domain.clone()),
                u16::try_from(handshake.port).unwrap_or(0),
            )
            .with_server(Some(ServerId::new(server_config.effective_id())))
            .with_legacy(true)
        })
        .await;
        match screened {
            Admission::Admitted => {}
            Admission::Denied(reason) => {
                send_legacy_kick(ctx.stream_mut(), &reason).await.ok();
                return Ok(());
            }
            Admission::Dropped => return Ok(()),
        }

        let attempt = LoginAttempt::pre_auth(ctx.client_ip, handshake.username.clone())
            .virtual_host(domain.clone())
            .server(ServerId::new(server_config.effective_id()));
        if let Some(refusal) = self.services.ban_manager.refuse(&attempt).await {
            admission::reject(
                &self.services.event_bus,
                ctx.client_addr(),
                Some(domain),
                refusal.reason,
            );
            send_legacy_kick(ctx.stream_mut(), &refusal.message)
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

    async fn read_legacy_handshake_data(
        &self,
        ctx: &mut ConnectionContext,
    ) -> Result<Vec<u8>, CoreError> {
        let mut data = Vec::with_capacity(256);

        data.push(0x02);

        let mut format_byte = [0u8; 1];
        ctx.stream_mut().read_exact(&mut format_byte).await?;
        data.push(format_byte[0]);

        if format_byte[0] == 0x00 {
            let mut low_byte = [0u8; 1];
            ctx.stream_mut().read_exact(&mut low_byte).await?;
            data.push(low_byte[0]);

            let str_len = u16::from_be_bytes([0x00, low_byte[0]]) as usize;
            let mut str_data = vec![0u8; str_len * 2];
            ctx.stream_mut().read_exact(&mut str_data).await?;
            data.extend_from_slice(&str_data);
        } else {
            self.read_legacy_string_into(ctx, &mut data).await?;
            self.read_legacy_string_into(ctx, &mut data).await?;
            let mut port = [0u8; 4];
            ctx.stream_mut().read_exact(&mut port).await?;
            data.extend_from_slice(&port);
        }

        Ok(data)
    }

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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    use infrarust_config::ServerAddress;
    use infrarust_transport::BackendConnector;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_util::sync::CancellationToken;

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
}
