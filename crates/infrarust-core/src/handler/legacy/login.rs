use infrarust_api::events::handshake::RejectReason;
use infrarust_api::services::ban_service::LoginAttempt;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::legacy::LegacyHandshakeRequest;

use super::LegacyHandler;
use crate::error::CoreError;
use crate::handler::forwarded::{
    Arrival, ForwardedLogin, Opening, PROXY_LOGIN_SERVER, Route, Wire,
};
use crate::handler::helpers::send_legacy_kick;
use crate::pipeline::admission;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};

impl LegacyHandler {
    pub(crate) async fn handle_login(
        &self,
        ctx: &mut ConnectionContext,
        request: &LegacyHandshakeRequest,
        raw: &[u8],
    ) -> Result<(), CoreError> {
        let domain = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .domain
            .clone();
        let routing = ctx.require_extension::<RoutingData>("RoutingData")?.clone();

        let attempt = LoginAttempt::pre_auth(ctx.client_ip, request.username.clone())
            .virtual_host(domain.clone())
            .server(ServerId::new(routing.config_id.clone()));
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

        let login = ForwardedLogin {
            services: &self.services,
            connector: &self.backend_connector,
            shutdown: &self.shutdown,
            wire: Wire::Legacy,
        };
        let Some(origin) = Route::forwarding(routing, |routing| login.select(ctx, routing)) else {
            tracing::debug!(%domain, "legacy login: the proxy runs the login on this server");
            admission::reject(
                &self.services.event_bus,
                ctx.client_addr(),
                Some(domain),
                RejectReason::ServerUnavailable,
            );
            send_legacy_kick(ctx.stream_mut(), &Component::text(PROXY_LOGIN_SERVER))
                .await
                .ok();
            return Ok(());
        };
        let arrival = Arrival {
            username: request.username.clone(),
            claimed_uuid: None,
            protocol_version: infrarust_api::types::ProtocolVersion::new(i32::from(
                request.protocol_version,
            )),
            domain,
        };
        let Some(ready) = login
            .open(ctx, arrival, origin, Opening::Legacy(raw))
            .await?
        else {
            return Ok(());
        };

        let server = ready.server.clone();
        tracing::info!(
            server = %server,
            username = %request.username,
            "legacy login: forwarding to backend"
        );

        let result = ready.forward(ctx.take_stream()).await;

        tracing::info!(
            server = %server,
            username = %request.username,
            c2b = result.client_to_backend,
            b2c = result.backend_to_client,
            reason = ?result.reason,
            "legacy session ended"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    use infrarust_config::ServerAddress;
    use infrarust_transport::BackendConnector;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::handler::legacy::hello;
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::loadbalancer::AddressConnectionCount;
    use crate::pipeline::types::LegacyHello;
    use crate::provider::ProviderId;
    use crate::status::StatusCache;

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

    fn lobby_handler(backend_addr: std::net::SocketAddr) -> Arc<LegacyHandler> {
        let services = test_proxy_services();
        services.domain_router.add(
            ProviderId::file("lobby"),
            toml::from_str(&format!(
                "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"{backend_addr}\"]\n"
            ))
            .unwrap(),
        );
        Arc::new(LegacyHandler::new(
            services,
            Arc::new(BackendConnector::new(
                std::time::Duration::from_secs(2),
                infrarust_config::KeepaliveConfig::default(),
            )),
            CancellationToken::new(),
            Arc::new(StatusCache::new(std::time::Duration::from_secs(5))),
        ))
    }

    async fn login_after_the_pipeline(
        handler: &LegacyHandler,
        ctx: &mut ConnectionContext,
    ) -> Result<(), CoreError> {
        hello::read_login_hello(ctx).await?;
        let (_, server_config, load_balancer) = handler
            .services
            .domain_router
            .resolve_route("lobby.test")
            .unwrap();
        ctx.extensions.insert(RoutingData {
            config_id: server_config.effective_id(),
            server_config,
            load_balancer,
        });
        let Some(LegacyHello::Login { request, raw }) =
            ctx.extensions.get::<LegacyHello>().cloned()
        else {
            panic!("a legacy login must leave its hello");
        };
        handler.handle_login(ctx, &request, &raw).await
    }

    async fn login_read_ahead(read_ahead: usize) {
        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let handler = lobby_handler(backend_listener.local_addr().unwrap());
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(proxy_listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server_side, _) = proxy_listener.accept().await.unwrap();

        let tail = legacy_login_tail("Notch", "lobby.test", 25565);
        let mut ctx = test_ctx(server_side).await;
        ctx.buffered_data.extend_from_slice(&tail[..read_ahead]);
        client.write_all(&tail[read_ahead..]).await.unwrap();
        let login = {
            let handler = Arc::clone(&handler);
            tokio::spawn(async move { login_after_the_pipeline(&handler, &mut ctx).await })
        };

        let (mut backend, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), backend_listener.accept())
                .await
                .expect("the login never reached the backend")
                .unwrap();
        let mut forwarded = vec![0u8; 1 + tail.len()];
        backend.read_exact(&mut forwarded).await.unwrap();
        assert_eq!(forwarded[0], 0x02, "read ahead {read_ahead}");
        assert_eq!(forwarded[1..], tail[..], "read ahead {read_ahead}");

        drop(client);
        drop(backend);
        login.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_legacy_login_read_ahead_whole_is_not_waited_for_again() {
        login_read_ahead(legacy_login_tail("Notch", "lobby.test", 25565).len()).await;
    }

    #[tokio::test]
    async fn a_legacy_login_read_ahead_in_part_is_completed_from_the_stream() {
        for read_ahead in [1, 2, 5, 13] {
            login_read_ahead(read_ahead).await;
        }
    }

    #[tokio::test]
    async fn a_legacy_login_is_counted_on_its_backend_for_the_whole_forward() {
        let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_addr = backend_listener.local_addr().unwrap();
        let address: ServerAddress = backend_addr.to_string().parse().unwrap();

        let handler = lobby_handler(backend_addr);
        let load = Arc::clone(&handler.services.backend_load);

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
            tokio::spawn(async move { login_after_the_pipeline(&handler, &mut ctx).await })
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
