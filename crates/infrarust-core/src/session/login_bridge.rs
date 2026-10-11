use std::collections::HashSet;
use std::time::Duration;

use tokio::time::Instant;

use infrarust_api::types::ServerId;
use infrarust_config::ServerConfig;
use infrarust_protocol::Packet;
use infrarust_protocol::codec::McBufReadExt;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::cookie::{CLoginCookieRequest, SLoginCookieResponse};
use infrarust_protocol::packets::login::{
    CEncryptionRequest, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CSetCompression,
    SLoginPluginResponse,
};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::forwarding::ForwardingData;
use crate::forwarding::velocity::{build_velocity_response, is_velocity_request};
use crate::session::backend_bridge::BackendBridge;
use crate::session::backend_login::acknowledge_login;
use crate::session::client_bridge::ClientBridge;
use crate::session::client_login::complete_login_within;
use crate::session::context::{SessionContext, SessionIo};
use crate::session::kick::BackendKick;
use crate::session::login_messages::{LoginMessages, Routed};

pub(crate) const CLIENT_WAIT_CAP: Duration = Duration::from_secs(5 * 60);

pub(crate) fn read_timeout(config: &ServerConfig) -> Duration {
    config
        .timeouts
        .as_ref()
        .map_or_else(infrarust_config::defaults::read_timeout, |t| t.read)
}

pub(crate) struct LoginClock {
    idle: Duration,
    waiting: bool,
    deadline: Instant,
}

impl LoginClock {
    pub(crate) fn start(idle: Duration) -> Self {
        Self {
            idle,
            waiting: false,
            deadline: Instant::now() + idle,
        }
    }

    pub(crate) const fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(crate) fn backend_spoke(&mut self, waiting: bool) {
        if !self.waiting {
            let allowed = if waiting { CLIENT_WAIT_CAP } else { self.idle };
            self.deadline = Instant::now() + allowed;
        }
        self.waiting = waiting;
    }

    pub(crate) fn client_answered(&mut self, waiting: bool) {
        if self.waiting && !waiting {
            self.deadline = Instant::now() + self.idle;
        }
        self.waiting = waiting;
    }

    pub(crate) fn expired(&self) -> CoreError {
        CoreError::Timeout(if self.waiting {
            "the client did not answer the backend's login messages in time".to_string()
        } else {
            "the backend did not finish the login".to_string()
        })
    }
}

pub(crate) async fn bridge_login(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    backend: &mut BackendBridge,
    server: &str,
    config: &ServerConfig,
    velocity: Option<(&ForwardingData, &[u8])>,
) -> Result<(), CoreError> {
    let idle = read_timeout(config);
    let mut relay = Relay {
        registry: ctx.registry(),
        version: ctx.version(),
        velocity,
        ids: LoginIds::resolve(ctx.registry(), ctx.version()),
        plugin_requests: HashSet::new(),
        cookie_requests: 0,
        messages: LoginMessages::new(ctx, ServerId::new(server)),
    };
    relay
        .run(ctx, &mut io.client, backend, LoginClock::start(idle))
        .await?;
    if !relay.plugin_requests.is_empty() {
        tracing::warn!(
            unanswered = relay.plugin_requests.len(),
            "the backend finished the login with login plugin requests still unanswered"
        );
    }
    acknowledge_login(ctx, backend).await?;
    if let Some(threshold) = backend.compression_threshold() {
        io.server_codec.notify_compression_change(threshold);
    }
    complete_login_within(ctx, io, idle)
        .await
        .map_err(client_gone)
}

struct LoginIds {
    set_compression: Option<i32>,
    success: Option<i32>,
    disconnect: Option<i32>,
    encryption: Option<i32>,
    plugin_request: Option<i32>,
    cookie_request: Option<i32>,
    plugin_response: Option<i32>,
    cookie_response: Option<i32>,
}

impl LoginIds {
    fn resolve(registry: &PacketRegistry, version: ProtocolVersion) -> Self {
        Self {
            set_compression: registry.get_packet_id::<CSetCompression>(version),
            success: registry.get_packet_id::<CLoginSuccess>(version),
            disconnect: registry.get_packet_id::<CLoginDisconnect>(version),
            encryption: registry.get_packet_id::<CEncryptionRequest>(version),
            plugin_request: registry.get_packet_id::<CLoginPluginRequest>(version),
            cookie_request: registry.get_packet_id::<CLoginCookieRequest>(version),
            plugin_response: registry.get_packet_id::<SLoginPluginResponse>(version),
            cookie_response: registry.get_packet_id::<SLoginCookieResponse>(version),
        }
    }
}

enum Event {
    Cancelled,
    Expired,
    Client(Result<Option<PacketFrame>, CoreError>),
    Backend(Result<Option<PacketFrame>, CoreError>),
}

enum Progress {
    Continue,
    LoggedIn,
}

struct Relay<'a> {
    registry: &'a PacketRegistry,
    version: ProtocolVersion,
    velocity: Option<(&'a ForwardingData, &'a [u8])>,
    ids: LoginIds,
    plugin_requests: HashSet<i32>,
    cookie_requests: usize,
    messages: LoginMessages<'a>,
}

impl Relay<'_> {
    fn waiting_on_client(&self) -> bool {
        !self.plugin_requests.is_empty() || self.cookie_requests > 0
    }

    async fn run(
        &mut self,
        ctx: &SessionContext<'_>,
        client: &mut ClientBridge,
        backend: &mut BackendBridge,
        mut clock: LoginClock,
    ) -> Result<(), CoreError> {
        let deadline = tokio::time::sleep_until(clock.deadline());
        tokio::pin!(deadline);
        loop {
            let event = tokio::select! {
                biased;
                () = ctx.token.cancelled() => Event::Cancelled,
                () = &mut deadline => Event::Expired,
                frame = backend.read_frame() => Event::Backend(frame),
                frame = client.read_frame() => Event::Client(frame),
            };
            match event {
                Event::Cancelled => {
                    return Err(CoreError::Rejected(
                        "the session ended during the backend login".to_string(),
                    ));
                }
                Event::Expired => return Err(clock.expired()),
                Event::Backend(Ok(Some(frame))) => {
                    if let Progress::LoggedIn = self.on_backend(frame, client, backend).await? {
                        return Ok(());
                    }
                    clock.backend_spoke(self.waiting_on_client());
                }
                Event::Backend(Ok(None)) => return Err(CoreError::ConnectionClosed),
                Event::Backend(Err(e)) => return Err(e),
                Event::Client(Ok(Some(frame))) => {
                    self.on_client(frame, backend).await?;
                    clock.client_answered(self.waiting_on_client());
                }
                Event::Client(Ok(None)) => return Err(CoreError::ClientLeft),
                Event::Client(Err(e)) => return Err(client_gone(e)),
            }
            if deadline.deadline() != clock.deadline() {
                deadline.as_mut().reset(clock.deadline());
            }
        }
    }

    async fn on_backend(
        &mut self,
        frame: PacketFrame,
        client: &mut ClientBridge,
        backend: &mut BackendBridge,
    ) -> Result<Progress, CoreError> {
        let id = Some(frame.id);
        if id == self.ids.success {
            if self.version.less_than(ProtocolVersion::V1_20_2) {
                backend.set_state(ConnectionState::Play);
            }
            tracing::debug!("consumed backend LoginSuccess");
            return Ok(Progress::LoggedIn);
        }
        if id == self.ids.set_compression {
            let packet = CSetCompression::decode(&mut frame.payload.as_ref(), self.version)?;
            backend.set_compression(packet.threshold.0);
            tracing::debug!(
                threshold = packet.threshold.0,
                "backend compression activated"
            );
        } else if id == self.ids.disconnect {
            return Err(CoreError::BackendKick(Box::new(BackendKick::new(
                frame,
                ConnectionState::Login,
                self.version,
            ))));
        } else if id == self.ids.encryption {
            return Err(backend.online_mode_refusal());
        } else if id == self.ids.plugin_request {
            let request = CLoginPluginRequest::decode(&mut frame.payload.as_ref(), self.version)?;
            if is_velocity_request(&request) {
                self.answer_velocity(&request, backend).await?;
                return Ok(Progress::Continue);
            }
            match self.messages.relay(&frame, &request).await {
                Routed::Client(replaced) => {
                    client
                        .write_frame(replaced.as_ref().unwrap_or(&frame))
                        .await
                        .map_err(client_gone)?;
                    self.plugin_requests.insert(request.message_id.0);
                    tracing::debug!(
                        channel = %request.channel,
                        message_id = request.message_id.0,
                        "relayed a login plugin request to the client"
                    );
                }
                Routed::Backend(answer) => backend.send_packet(&answer, self.registry).await?,
            }
        } else if id == self.ids.cookie_request {
            client.write_frame(&frame).await.map_err(client_gone)?;
            self.cookie_requests += 1;
            tracing::debug!("relayed a login cookie request to the client");
        } else {
            tracing::debug!(id = frame.id, "ignoring a backend packet during the login");
        }
        Ok(Progress::Continue)
    }

    async fn answer_velocity(
        &self,
        request: &CLoginPluginRequest,
        backend: &mut BackendBridge,
    ) -> Result<(), CoreError> {
        let response = match self.velocity {
            Some((data, secret)) => {
                let response = build_velocity_response(request, data, secret)?;
                tracing::info!("velocity forwarding applied (auto-detected)");
                response
            }
            None => {
                tracing::debug!(
                    "backend requested velocity forwarding but no secret is configured, \
                     responding 'not understood'"
                );
                SLoginPluginResponse {
                    message_id: request.message_id,
                    successful: false,
                    data: Vec::new(),
                }
            }
        };
        backend.send_packet(&response, self.registry).await
    }

    async fn on_client(
        &mut self,
        frame: PacketFrame,
        backend: &mut BackendBridge,
    ) -> Result<(), CoreError> {
        let id = Some(frame.id);
        if id == self.ids.plugin_response {
            let Ok(message_id) = frame.payload.as_ref().read_var_int() else {
                tracing::debug!("dropping a malformed login plugin response from the client");
                return Ok(());
            };
            if !self.plugin_requests.remove(&message_id.0) {
                tracing::debug!(
                    message_id = message_id.0,
                    "dropping a login plugin response the backend did not ask for"
                );
                return Ok(());
            }
            let replaced = self.messages.response(&frame).await;
            backend
                .write_frame(replaced.as_ref().unwrap_or(&frame))
                .await
        } else if id == self.ids.cookie_response {
            if self.cookie_requests == 0 {
                tracing::debug!("dropping a login cookie response the backend did not ask for");
                return Ok(());
            }
            self.cookie_requests -= 1;
            backend.write_frame(&frame).await
        } else {
            tracing::debug!(
                id = frame.id,
                "ignoring a client packet during the backend login"
            );
            Ok(())
        }
    }
}

fn client_gone(error: CoreError) -> CoreError {
    if error.is_expected_disconnect() {
        tracing::debug!("the client left during the backend login: {error}");
    } else {
        tracing::warn!("the client broke off its login: {error}");
    }
    CoreError::ClientLeft
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use bytes::Bytes;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    use infrarust_api::event::ConnectionState as ApiState;
    use infrarust_api::filter::ConnectionSide;
    use infrarust_api::types::ServerId;
    use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig};
    use infrarust_protocol::packets::login::SLoginAcknowledged;
    use infrarust_protocol::{ProtocolError, VarInt};

    use super::*;
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::player::PlayerSession;
    use crate::services::ProxyServices;
    use crate::session::backend_login::{Login, connect_backend};
    use crate::session::context::test_helpers::{test_connector, test_context, test_io};
    use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};
    use crate::session::test_peer::{
        Notice, Notices, V, backend_login, backend_success, client_pair, finish_backend_login,
        finish_client_login, oversized_payload, plugin_request, plugin_response, server_config,
        var_int, velocity_services, with_handshake,
    };

    async fn bridge(
        ctx: &SessionContext<'_>,
        io: &mut SessionIo,
        config: &ServerConfig,
    ) -> Result<BackendBridge, CoreError> {
        let addresses = vec![config.addresses[0].address.clone()];
        connect_backend(ctx, "origin", config, &addresses, Login::Bridged(io)).await
    }

    fn services_with(config: &str) -> ProxyServices {
        let mut services = test_proxy_services();
        services.config = Arc::new(toml::from_str(config).unwrap());
        services
    }

    #[tokio::test]
    async fn pipelined_requests_answered_out_of_order_reach_the_backend_unchanged() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            let mut requests = Vec::new();
            for (id, channel) in [
                (-100, "example:handshake"),
                (-101, "example:data"),
                (7, "fml:loginwrapper"),
            ] {
                requests.push(conn.send(&plugin_request(id, channel, b"ask")).await);
            }
            let mut answers = Vec::new();
            for _ in 0..3 {
                answers.push(conn.frame().await);
            }
            finish_backend_login(&mut conn).await;
            (requests, answers, conn)
        });
        let client_side = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..3 {
                requests.push(peer.frame().await);
            }
            peer.send(&plugin_response(42, b"stray")).await;
            let mut answers = Vec::new();
            for id in [7, -101, -100] {
                answers.push(
                    peer.send(&plugin_response(id, format!("answer {id}").as_bytes()))
                        .await,
                );
            }
            finish_client_login(&mut peer).await;
            (requests, answers, peer)
        });

        let backend_bridge = bridge(&ctx, &mut io, &config).await.unwrap();
        let (sent_requests, received_answers, _conn) = backend_side.await.unwrap();
        let (received_requests, sent_answers, _peer) = client_side.await.unwrap();

        for (sent, received) in sent_requests.iter().zip(&received_requests) {
            assert_eq!(sent.id, received.id);
            assert_eq!(sent.payload, received.payload);
        }
        assert_eq!(received_requests[0].payload[..5], var_int(-100)[..]);
        assert_eq!(received_requests[1].payload[..5], var_int(-101)[..]);
        for (sent, received) in sent_answers.iter().zip(&received_answers) {
            assert_eq!(sent.id, received.id);
            assert_eq!(sent.payload, received.payload);
        }
        assert_eq!(received_answers[1].payload[..5], var_int(-101)[..]);
        assert_eq!(backend_bridge.state, ConnectionState::Config);
    }

    #[tokio::test]
    async fn the_proxy_answers_velocity_while_a_custom_request_goes_to_the_client() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let services = velocity_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(1, "velocity:player_info", &[4]))
                .await;
            conn.send(&plugin_request(2, "fml:loginwrapper", b"mods"))
                .await;
            let mut answers = Vec::new();
            for _ in 0..2 {
                answers.push(conn.expect::<SLoginPluginResponse>().await);
            }
            finish_backend_login(&mut conn).await;
            (answers, conn)
        });
        let client_side = tokio::spawn(async move {
            let request = peer.expect::<CLoginPluginRequest>().await;
            peer.send(&plugin_response(request.message_id.0, b"client"))
                .await;
            finish_client_login(&mut peer).await;
            (request, peer)
        });

        bridge(&ctx, &mut io, &config).await.unwrap();
        let (answers, _conn) = backend_side.await.unwrap();
        let (request, _peer) = client_side.await.unwrap();

        assert_eq!(request.channel, "fml:loginwrapper");
        assert_eq!(request.message_id.0, 2);
        let velocity = answers.iter().find(|a| a.message_id.0 == 1).unwrap();
        assert!(velocity.successful && !velocity.data.is_empty());
        let relayed = answers.iter().find(|a| a.message_id.0 == 2).unwrap();
        assert_eq!(relayed.data, b"client");
    }

    #[tokio::test]
    async fn the_client_hears_nothing_until_the_backend_accepts_the_login() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);
        let (go, released) = oneshot::channel::<()>();

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(1, "fml:loginwrapper", b"mods"))
                .await;
            conn.expect::<SLoginPluginResponse>().await;
            released.await.unwrap();
            conn.send(&CSetCompression {
                threshold: VarInt(64),
            })
            .await;
            conn.set_compression(64);
            finish_backend_login(&mut conn).await;
            conn
        });
        let client_side = tokio::spawn(async move {
            peer.expect::<CLoginPluginRequest>().await;
            peer.send(&plugin_response(1, b"ok")).await;
            let early = tokio::time::timeout(Duration::from_millis(200), peer.next()).await;
            assert!(early.is_err(), "the client heard {early:?} too early");
            go.send(()).unwrap();
            let threshold = finish_client_login(&mut peer).await;
            (threshold, peer)
        });

        bridge(&ctx, &mut io, &config).await.unwrap();
        let _conn = backend_side.await.unwrap();
        let (threshold, _peer) = client_side.await.unwrap();

        assert_eq!(
            threshold, 256,
            "the client gets the proxy threshold, not the backend's"
        );
    }

    #[tokio::test]
    async fn a_login_cookie_request_is_relayed_both_ways() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&CLoginCookieRequest {
                key: "example:token".to_string(),
            })
            .await;
            let answer = conn.frame().await;
            finish_backend_login(&mut conn).await;
            (answer, conn)
        });
        let client_side = tokio::spawn(async move {
            let request = peer.expect::<CLoginCookieRequest>().await;
            let answer = peer
                .send(&SLoginCookieResponse {
                    key: request.key.clone(),
                    payload: Some(b"cookie".to_vec()),
                })
                .await;
            finish_client_login(&mut peer).await;
            (request, answer, peer)
        });

        bridge(&ctx, &mut io, &config).await.unwrap();
        let (received, _conn) = backend_side.await.unwrap();
        let (request, sent, _peer) = client_side.await.unwrap();

        assert_eq!(request.key, "example:token");
        assert_eq!(received.id, sent.id);
        assert_eq!(received.payload, sent.payload);
    }

    #[tokio::test]
    async fn an_online_mode_backend_fails_the_login_at_once() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, _peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&CEncryptionRequest {
                server_id: String::new(),
                public_key: vec![1; 162],
                verify_token: vec![2; 4],
                should_authenticate: true,
            })
            .await;
            conn.next().await
        });

        let result =
            tokio::time::timeout(Duration::from_secs(5), bridge(&ctx, &mut io, &config)).await;

        let Ok(Err(CoreError::Auth(message))) = result else {
            panic!(
                "expected an immediate refusal, got {:?}",
                result.map(|r| r.err())
            );
        };
        assert!(message.contains("online-mode=false"), "{message}");
        assert!(backend_side.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_silent_backend_times_out_after_the_read_timeout() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "[timeouts]\nread = \"150ms\"");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, _peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.next().await
        });

        let result =
            tokio::time::timeout(Duration::from_secs(5), bridge(&ctx, &mut io, &config)).await;

        assert!(
            matches!(result, Ok(Err(CoreError::Timeout(_)))),
            "{:?}",
            result.map(|r| r.err())
        );
        assert!(backend_side.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_client_may_answer_after_the_read_timeout() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "[timeouts]\nread = \"150ms\"");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(-100, "example:handshake", b"hello"))
                .await;
            conn.expect::<SLoginPluginResponse>().await;
            finish_backend_login(&mut conn).await;
            conn
        });
        let client_side = tokio::spawn(async move {
            peer.expect::<CLoginPluginRequest>().await;
            tokio::time::sleep(Duration::from_millis(600)).await;
            peer.send(&plugin_response(-100, b"later")).await;
            finish_client_login(&mut peer).await;
            peer
        });

        bridge(&ctx, &mut io, &config).await.unwrap();
        let _conn = backend_side.await.unwrap();
        let _peer = client_side.await.unwrap();
    }

    #[tokio::test]
    async fn a_client_that_never_answers_ends_the_login_at_the_wait_cap() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "[timeouts]\nread = \"150ms\"");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(-100, "example:handshake", b"hello"))
                .await;
            conn.next().await
        });
        let client_side = tokio::spawn(async move {
            peer.expect::<CLoginPluginRequest>().await;
            tokio::time::pause();
            peer.next().await
        });

        let started = Instant::now();
        let result = bridge(&ctx, &mut io, &config).await;
        let waited = started.elapsed();
        drop(io);

        assert!(
            matches!(result, Err(CoreError::Timeout(_))),
            "{:?}",
            result.err()
        );
        assert!(waited >= CLIENT_WAIT_CAP, "ended after {waited:?}");
        assert!(backend_side.await.unwrap().is_none());
        assert!(client_side.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn client_packets_do_not_hold_off_the_backend_read_timeout() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "[timeouts]\nread = \"150ms\"");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);
        let (stop, stopped) = oneshot::channel::<()>();

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.next().await
        });
        let client_side = tokio::spawn(async move {
            let junk = async {
                loop {
                    peer.send(&plugin_response(42, b"junk")).await;
                    peer.send_frame(&PacketFrame::new(0x7F, Bytes::from_static(b"?")))
                        .await;
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
            };
            tokio::select! {
                _ = stopped => {}
                _ = junk => {}
            }
            peer
        });

        let started = Instant::now();
        let result =
            tokio::time::timeout(Duration::from_secs(5), bridge(&ctx, &mut io, &config)).await;
        let waited = started.elapsed();
        stop.send(()).unwrap();
        let _peer = client_side.await.unwrap();

        assert!(
            matches!(result, Ok(Err(CoreError::Timeout(_)))),
            "{:?}",
            result.map(|r| r.err())
        );
        assert!(waited < Duration::from_secs(2), "ended after {waited:?}");
        assert!(backend_side.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_client_that_never_acknowledges_the_login_is_let_go() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "[timeouts]\nread = \"150ms\"");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            finish_backend_login(&mut conn).await;
            conn
        });
        let client_side = tokio::spawn(async move {
            let compression = peer.expect::<CSetCompression>().await;
            peer.set_compression(compression.threshold.0);
            peer.expect::<CLoginSuccess>().await;
            peer
        });

        let result =
            tokio::time::timeout(Duration::from_secs(5), bridge(&ctx, &mut io, &config)).await;
        let _conn = backend_side.await.unwrap();
        let _peer = client_side.await.unwrap();

        assert!(
            matches!(result, Ok(Err(CoreError::ClientLeft))),
            "{:?}",
            result.map(|r| r.err())
        );
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    struct PerEventInterest;

    impl tracing::Subscriber for PerEventInterest {
        fn register_callsite(
            &self,
            _: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            tracing::subscriber::Interest::sometimes()
        }

        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            false
        }

        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

        fn event(&self, _: &tracing::Event<'_>) {}

        fn enter(&self, _: &tracing::span::Id) {}

        fn exit(&self, _: &tracing::span::Id) {}
    }

    static PER_EVENT_INTEREST: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();

    #[tokio::test]
    async fn stray_client_answers_are_not_logged_as_warnings() {
        PER_EVENT_INTEREST.get_or_init(|| tracing::Dispatch::new(PerEventInterest));
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(captured.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let services = test_proxy_services();
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(-100, "example:handshake", b"hello"))
                .await;
            let answer = conn.expect::<SLoginPluginResponse>().await;
            finish_backend_login(&mut conn).await;
            (answer, conn)
        });
        let client_side = tokio::spawn(async move {
            let request = peer.expect::<CLoginPluginRequest>().await;
            let response = peer.id::<SLoginPluginResponse>();
            for _ in 0..20 {
                peer.send_frame(&PacketFrame::new(response, Bytes::new()))
                    .await;
                peer.send(&plugin_response(42, b"stray")).await;
                peer.send(&SLoginCookieResponse {
                    key: "example:token".to_string(),
                    payload: None,
                })
                .await;
            }
            peer.send(&plugin_response(request.message_id.0, b"real"))
                .await;
            finish_client_login(&mut peer).await;
            peer
        });

        bridge(&ctx, &mut io, &config).await.unwrap();
        let (answer, _conn) = backend_side.await.unwrap();
        let _peer = client_side.await.unwrap();

        assert_eq!(answer.data, b"real");
        let logged = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(
            !logged.contains("did not ask for") && !logged.contains("malformed"),
            "{logged}"
        );
    }

    #[tokio::test]
    async fn each_codec_chain_hears_its_own_threshold_and_every_state() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let services = test_proxy_services();
        let notices = Notices::install(&services);
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&CSetCompression {
                threshold: VarInt(64),
            })
            .await;
            conn.set_compression(64);
            finish_backend_login(&mut conn).await;
            conn.send(&CFinishConfig).await;
            conn.expect::<SAcknowledgeFinishConfig>().await;
            conn
        });
        let client_side = tokio::spawn(async move {
            finish_client_login(&mut peer).await;
            peer.expect::<CFinishConfig>().await;
            peer.send(&SAcknowledgeFinishConfig).await;
        });

        let mut backend_bridge = bridge(&ctx, &mut io, &config).await.unwrap();
        let server = ServerId::new("origin");
        let mut join = None;
        let loop_side = proxy_loop(&ctx, &mut io, &mut backend_bridge, &server, &mut join);
        let (outcome, sent) = tokio::join!(loop_side, client_side);
        sent.unwrap();
        let _conn = backend_side.await.unwrap();

        assert!(
            matches!(outcome, ProxyLoopOutcome::ClientDisconnected),
            "{outcome:?}"
        );
        assert_eq!(
            notices.of(ConnectionSide::ClientSide),
            [
                Notice::Compression(256),
                Notice::State(ApiState::Configuration),
                Notice::State(ApiState::Play),
            ]
        );
        assert_eq!(
            notices.of(ConnectionSide::ServerSide),
            [
                Notice::Compression(64),
                Notice::State(ApiState::Configuration),
                Notice::State(ApiState::Play),
            ]
        );
    }

    async fn oversized_backend(backend: TcpListener) {
        let (mut conn, _) = backend_login(&backend).await;
        conn.send(&CSetCompression {
            threshold: VarInt(256),
        })
        .await;
        conn.set_compression(256);
        conn.send(&backend_success()).await;
        conn.expect::<SLoginAcknowledged>().await;
        conn.send_frame(&PacketFrame::new(0x0F, oversized_payload()))
            .await;
        conn.next().await;
    }

    #[tokio::test]
    async fn a_compressed_client_link_carries_a_packet_over_two_mebibytes() {
        let services = test_proxy_services();
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);
        let expected = oversized_payload().len();

        let backend_side = tokio::spawn(oversized_backend(backend));
        let client_side = tokio::spawn(async move {
            let threshold = finish_client_login(&mut peer).await;
            let frame = peer.frame().await;
            (threshold, frame.id, frame.payload.len())
        });

        let mut backend_bridge = bridge(&ctx, &mut io, &config).await.unwrap();
        let server = ServerId::new("origin");
        let mut join = None;
        let loop_side = proxy_loop(&ctx, &mut io, &mut backend_bridge, &server, &mut join);
        let (outcome, received) = tokio::join!(loop_side, client_side);
        let (threshold, id, len) = received.unwrap();
        drop(backend_bridge);
        backend_side.await.unwrap();

        assert_eq!(threshold, 256);
        assert_eq!((id, len), (0x0F, expected));
        assert!(
            matches!(outcome, ProxyLoopOutcome::ClientDisconnected),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn without_client_compression_the_same_packet_ends_the_session() {
        let services = services_with("compression_threshold = -1");
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&backend, "");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(oversized_backend(backend));
        let client_side = tokio::spawn(async move {
            peer.expect::<CLoginSuccess>().await;
            peer.send(&SLoginAcknowledged).await;
            peer.next().await.map(|frame| frame.id)
        });

        let mut backend_bridge = bridge(&ctx, &mut io, &config).await.unwrap();
        assert_eq!(io.client.compression_threshold(), None);
        let server = ServerId::new("origin");
        let outcome = proxy_loop(&ctx, &mut io, &mut backend_bridge, &server, &mut None).await;
        drop(io);
        drop(backend_bridge);
        let received = client_side.await.unwrap();
        backend_side.await.unwrap();

        assert!(
            matches!(
                outcome,
                ProxyLoopOutcome::Error(CoreError::Protocol(ProtocolError::TooLarge { .. }))
            ),
            "{outcome:?}"
        );
        assert_eq!(received, None);
    }
}
