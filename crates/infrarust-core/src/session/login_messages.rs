use std::collections::HashMap;
use std::sync::Arc;

use bytes::{BufMut, Bytes, BytesMut};

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::messaging::{
    LoginPluginRequestEvent, LoginPluginRequestResult, LoginPluginResponseEvent,
    LoginPluginResponseResult,
};
use infrarust_api::player::Player;
use infrarust_api::types::ServerId;
use infrarust_protocol::VarInt;
use infrarust_protocol::codec::McBufReadExt;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::login::{CLoginPluginRequest, SLoginPluginResponse};
use infrarust_protocol::version::ProtocolVersion;

use crate::forwarding::velocity::is_velocity_request;
use crate::player::PlayerSession;
use crate::services::ProxyServices;
use crate::session::context::SessionContext;

pub(crate) enum Routed {
    Client(Option<PacketFrame>),
    Backend(SLoginPluginResponse),
}

struct Watch {
    requests: bool,
    responses: bool,
}

pub(crate) struct LoginMessages<'a> {
    services: &'a ProxyServices,
    session: &'a Arc<PlayerSession>,
    server: ServerId,
    version: ProtocolVersion,
    relayed: HashMap<i32, String>,
}

impl<'a> LoginMessages<'a> {
    pub(crate) fn new(ctx: &'a SessionContext<'_>, server: ServerId) -> Self {
        Self {
            services: ctx.services,
            session: &ctx.session,
            server,
            version: ctx.version(),
            relayed: HashMap::new(),
        }
    }

    pub(crate) async fn relay(
        &mut self,
        frame: &PacketFrame,
        request: &CLoginPluginRequest,
    ) -> Routed {
        let Some(watch) = self.watch(request) else {
            return Routed::Client(None);
        };
        let result = if watch.requests {
            self.decide(frame, request, true).await
        } else {
            LoginPluginRequestResult::Forward
        };
        let replaced = match result {
            LoginPluginRequestResult::Respond(data) => {
                return Routed::Backend(answered(request, data));
            }
            LoginPluginRequestResult::Replace(data) => {
                tracing::debug!(
                    channel = %request.channel,
                    message_id = request.message_id.0,
                    "a plugin replaced a login plugin request"
                );
                Some(with_data(frame, &request.data, &data))
            }
            _ => None,
        };
        if watch.responses {
            self.relayed
                .insert(request.message_id.0, request.channel.clone());
        }
        Routed::Client(replaced)
    }

    pub(crate) async fn answer(
        &self,
        frame: &PacketFrame,
        request: &CLoginPluginRequest,
    ) -> SLoginPluginResponse {
        if self.watch(request).is_some_and(|watch| watch.requests)
            && let LoginPluginRequestResult::Respond(data) =
                self.decide(frame, request, false).await
        {
            return answered(request, data);
        }
        tracing::debug!(
            channel = %request.channel,
            "responding 'not understood' to LoginPluginRequest"
        );
        response_to(request.message_id, None)
    }

    pub(crate) async fn response(&mut self, frame: &PacketFrame) -> Option<PacketFrame> {
        if self.relayed.is_empty() {
            return None;
        }
        let message_id = frame.payload.as_ref().read_var_int().ok()?.0;
        let channel = self.relayed.remove(&message_id)?;
        let response = match SLoginPluginResponse::decode(&mut frame.payload.as_ref(), self.version)
        {
            Ok(response) => response,
            Err(e) => {
                tracing::debug!(%channel, message_id, "could not read a login plugin response: {e}");
                return None;
            }
        };
        let start = data_start(frame, &response.data);
        let event = LoginPluginResponseEvent::new(
            self.player(),
            self.server.clone(),
            message_id,
            channel,
            response.successful,
            frame.payload.slice(start..),
        );
        let event = self.services.event_bus.fire(event).await;
        let LoginPluginResponseResult::Replace { successful, data } = event.result() else {
            return None;
        };
        tracing::debug!(
            channel = %event.channel,
            message_id,
            "a plugin replaced a login plugin response"
        );
        let flag = start.saturating_sub(1);
        let mut payload = BytesMut::with_capacity(flag + 1 + data.len());
        payload.extend_from_slice(&frame.payload[..flag]);
        payload.put_u8(u8::from(*successful));
        payload.extend_from_slice(data);
        Some(PacketFrame::new(frame.id, payload.freeze()))
    }

    fn watch(&self, request: &CLoginPluginRequest) -> Option<Watch> {
        if is_velocity_request(request) {
            return None;
        }
        self.services
            .plugin_messaging
            .channels()
            .lookup(&request.channel)?;
        let bus = &self.services.event_bus;
        let watch = Watch {
            requests: bus.has_listeners::<LoginPluginRequestEvent>(),
            responses: bus.has_listeners::<LoginPluginResponseEvent>(),
        };
        (watch.requests || watch.responses).then_some(watch)
    }

    async fn decide(
        &self,
        frame: &PacketFrame,
        request: &CLoginPluginRequest,
        client_reachable: bool,
    ) -> LoginPluginRequestResult {
        let event = LoginPluginRequestEvent::new(
            self.player(),
            self.server.clone(),
            request.message_id.0,
            request.channel.clone(),
            frame.payload.slice(data_start(frame, &request.data)..),
            client_reachable,
        );
        self.services.event_bus.fire(event).await.result().clone()
    }

    fn player(&self) -> Arc<dyn Player> {
        Arc::clone(self.session) as Arc<dyn Player>
    }
}

fn answered(request: &CLoginPluginRequest, data: Option<Bytes>) -> SLoginPluginResponse {
    tracing::debug!(
        channel = %request.channel,
        message_id = request.message_id.0,
        understood = data.is_some(),
        "a plugin answered a login plugin request"
    );
    response_to(request.message_id, data)
}

fn response_to(message_id: VarInt, data: Option<Bytes>) -> SLoginPluginResponse {
    SLoginPluginResponse {
        message_id,
        successful: data.is_some(),
        data: data.map(Vec::from).unwrap_or_default(),
    }
}

fn data_start(frame: &PacketFrame, data: &[u8]) -> usize {
    frame.payload.len().saturating_sub(data.len())
}

fn with_data(frame: &PacketFrame, old: &[u8], data: &[u8]) -> PacketFrame {
    let head = data_start(frame, old);
    let mut payload = BytesMut::with_capacity(head + data.len());
    payload.extend_from_slice(&frame.payload[..head]);
    payload.extend_from_slice(data);
    PacketFrame::new(frame.id, payload.freeze())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use tokio::net::TcpListener;

    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::events::messaging::LoginPluginRequestResult::{Forward, Replace, Respond};
    use infrarust_api::messaging::{ChannelId, ChannelRegistrar};
    use infrarust_api::types::PlayerId;
    use infrarust_protocol::packets::login::{CLoginSuccess, SLoginAcknowledged};

    use super::*;
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::plugin_messaging::TrackingChannelRegistrar;
    use crate::session::backend_login::{Login, connect_backend};
    use crate::session::context::test_helpers::{test_connector, test_context, test_io};
    use crate::session::proxy_loop::{ProxyLoopOutcome, proxy_loop};
    use crate::session::test_peer::{
        Peer, V, backend_login, client_pair, finish_backend_login, finish_client_login,
        plugin_request, plugin_response, server_config, var_int, velocity_services, with_handshake,
        with_login_start,
    };

    const HANDSHAKE: &str = "example:handshake";
    const DATA: &str = "example:data";
    const FORGE: &str = "fml:loginwrapper";
    const VELOCITY: &str = "velocity:player_info";

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SeenRequest {
        player: PlayerId,
        server: ServerId,
        message_id: i32,
        channel: String,
        data: Bytes,
        client_reachable: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SeenResponse {
        player: PlayerId,
        server: ServerId,
        message_id: i32,
        channel: String,
        successful: bool,
        data: Bytes,
    }

    type Seen<T> = Arc<Mutex<Vec<T>>>;

    fn seen<T: Clone>(log: &Seen<T>) -> Vec<T> {
        log.lock().unwrap().clone()
    }

    fn register(services: &ProxyServices, names: &[&str]) {
        let registrar =
            TrackingChannelRegistrar::new(Arc::clone(services.plugin_messaging.channels()), "test");
        for name in names {
            registrar.register(ChannelId::modern(name).unwrap());
        }
    }

    fn on_requests(
        services: &ProxyServices,
        decide: impl Fn(&str) -> LoginPluginRequestResult + Send + Sync + 'static,
    ) -> Seen<SeenRequest> {
        let log = Seen::default();
        let writer = Arc::clone(&log);
        let bus: &dyn EventBus = services.event_bus.as_ref();
        bus.subscribe::<LoginPluginRequestEvent, _>(EventPriority::NORMAL, move |event| {
            writer.lock().unwrap().push(SeenRequest {
                player: event.player.id(),
                server: event.server.clone(),
                message_id: event.message_id,
                channel: event.channel.clone(),
                data: event.data.clone(),
                client_reachable: event.client_reachable,
            });
            event.set_result(decide(&event.channel));
        });
        log
    }

    fn on_responses(
        services: &ProxyServices,
        decide: impl Fn(&str) -> LoginPluginResponseResult + Send + Sync + 'static,
    ) -> Seen<SeenResponse> {
        let log = Seen::default();
        let writer = Arc::clone(&log);
        let bus: &dyn EventBus = services.event_bus.as_ref();
        bus.subscribe::<LoginPluginResponseEvent, _>(EventPriority::NORMAL, move |event| {
            writer.lock().unwrap().push(SeenResponse {
                player: event.player.id(),
                server: event.server.clone(),
                message_id: event.message_id,
                channel: event.channel.clone(),
                successful: event.successful,
                data: event.data.clone(),
            });
            event.set_result(decide(&event.channel));
        });
        log
    }

    const fn pass(_: &str) -> LoginPluginResponseResult {
        LoginPluginResponseResult::Forward
    }

    type Answer = fn(&CLoginPluginRequest) -> SLoginPluginResponse;

    fn from_client(request: &CLoginPluginRequest) -> SLoginPluginResponse {
        plugin_response(request.message_id.0, b"from client")
    }

    fn handshake_only_client(request: &CLoginPluginRequest) -> SLoginPluginResponse {
        if request.channel == HANDSHAKE {
            plugin_response(request.message_id.0, b"ready")
        } else {
            response_to(request.message_id, None)
        }
    }

    struct Script {
        requests: Vec<CLoginPluginRequest>,
        reaching_client: usize,
        answer: Answer,
    }

    impl Script {
        fn new(requests: Vec<CLoginPluginRequest>, reaching_client: usize) -> Self {
            Self {
                requests,
                reaching_client,
                answer: from_client,
            }
        }
    }

    struct Outcome {
        player: PlayerId,
        sent: Vec<PacketFrame>,
        at_client: Vec<PacketFrame>,
        answered: Vec<PacketFrame>,
        at_backend: Vec<PacketFrame>,
    }

    impl Outcome {
        fn backend_got(&self, message_id: i32) -> SLoginPluginResponse {
            self.at_backend
                .iter()
                .map(|frame| SLoginPluginResponse::decode(&mut frame.payload.as_ref(), V).unwrap())
                .find(|response| response.message_id.0 == message_id)
                .unwrap_or_else(|| panic!("the backend got no answer to {message_id}"))
        }

        fn client_got(&self, message_id: i32) -> CLoginPluginRequest {
            self.at_client
                .iter()
                .map(|frame| CLoginPluginRequest::decode(&mut frame.payload.as_ref(), V).unwrap())
                .find(|request| request.message_id.0 == message_id)
                .unwrap_or_else(|| panic!("the client got no request {message_id}"))
        }

        fn request(
            &self,
            message_id: i32,
            channel: &str,
            data: &[u8],
            client_reachable: bool,
        ) -> SeenRequest {
            SeenRequest {
                player: self.player,
                server: ServerId::new("origin"),
                message_id,
                channel: channel.to_string(),
                data: Bytes::copy_from_slice(data),
                client_reachable,
            }
        }

        fn response(
            &self,
            message_id: i32,
            channel: &str,
            successful: bool,
            data: &[u8],
        ) -> SeenResponse {
            SeenResponse {
                player: self.player,
                server: ServerId::new("origin"),
                message_id,
                channel: channel.to_string(),
                successful,
                data: Bytes::copy_from_slice(data),
            }
        }
    }

    fn wire(frames: &[PacketFrame]) -> Vec<(i32, Bytes)> {
        frames
            .iter()
            .map(|frame| (frame.id, frame.payload.clone()))
            .collect()
    }

    #[derive(Debug, Clone, Copy)]
    enum Path {
        Bridged,
        Relayed,
    }

    const CLIENT_PATHS: [Path; 2] = [Path::Bridged, Path::Relayed];

    async fn play_backend(
        listener: TcpListener,
        requests: Vec<CLoginPluginRequest>,
    ) -> (Vec<PacketFrame>, Vec<PacketFrame>, Peer) {
        let (mut conn, _) = backend_login(&listener).await;
        let mut sent = Vec::new();
        for request in &requests {
            sent.push(conn.send(request).await);
        }
        let mut received = Vec::new();
        for _ in 0..requests.len() {
            received.push(conn.frame().await);
        }
        finish_backend_login(&mut conn).await;
        (sent, received, conn)
    }

    async fn answer_requests(
        peer: &mut Peer,
        count: usize,
        answer: Answer,
    ) -> (Vec<PacketFrame>, Vec<PacketFrame>) {
        let mut received = Vec::new();
        for _ in 0..count {
            let frame = peer.frame().await;
            assert_eq!(frame.id, peer.id::<CLoginPluginRequest>());
            received.push(frame);
        }
        let mut answered = Vec::new();
        for frame in received.iter().rev() {
            let request = CLoginPluginRequest::decode(&mut frame.payload.as_ref(), V).unwrap();
            answered.push(peer.send(&answer(&request)).await);
        }
        (received, answered)
    }

    async fn run(path: Path, services: &ProxyServices, script: Script) -> Outcome {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&listener, "");
        let addresses = vec![config.addresses[0].address.clone()];
        let connector = test_connector();
        let (session, _commands) = PlayerSession::new_test(true);
        let player = session.id();
        let ctx = with_login_start(test_context(services, &connector, session));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);
        let Script {
            requests,
            reaching_client,
            answer,
        } = script;

        let backend_side = tokio::spawn(play_backend(listener, requests));
        let client_side = tokio::spawn(async move {
            let exchanged = answer_requests(&mut peer, reaching_client, answer).await;
            match path {
                Path::Bridged => {
                    finish_client_login(&mut peer).await;
                }
                Path::Relayed => {
                    peer.expect::<CLoginSuccess>().await;
                    peer.send(&SLoginAcknowledged).await;
                }
            }
            exchanged
        });

        match path {
            Path::Bridged => {
                connect_backend(&ctx, "origin", &config, &addresses, Login::Bridged(&mut io))
                    .await
                    .unwrap();
            }
            Path::Relayed => {
                let mut backend =
                    connect_backend(&ctx, "origin", &config, &addresses, Login::Relay)
                        .await
                        .unwrap();
                let server = ServerId::new("origin");
                let ended = proxy_loop(&ctx, &mut io, &mut backend, &server, &mut None).await;
                assert!(
                    matches!(ended, ProxyLoopOutcome::ClientDisconnected),
                    "{ended:?}"
                );
            }
        }
        let (sent, at_backend, _conn) = backend_side.await.unwrap();
        let (at_client, answered) = client_side.await.unwrap();
        Outcome {
            player,
            sent,
            at_client,
            answered,
            at_backend,
        }
    }

    async fn proxied(services: &ProxyServices, requests: Vec<CLoginPluginRequest>) -> Outcome {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = server_config(&listener, "");
        let addresses = vec![config.addresses[0].address.clone()];
        let connector = test_connector();
        let (session, _commands) = PlayerSession::new_test(true);
        let player = session.id();
        let ctx = with_handshake(test_context(services, &connector, session));

        let backend_side = tokio::spawn(play_backend(listener, requests));
        connect_backend(&ctx, "origin", &config, &addresses, Login::Proxied)
            .await
            .unwrap();
        let (sent, at_backend, _conn) = backend_side.await.unwrap();
        Outcome {
            player,
            sent,
            at_client: Vec::new(),
            answered: Vec::new(),
            at_backend,
        }
    }

    fn not_understood(answer: &SLoginPluginResponse) -> bool {
        !answer.successful && answer.data.is_empty()
    }

    #[tokio::test]
    async fn a_forwarded_request_and_its_answer_cross_unchanged_and_fire_both_events() {
        for path in CLIENT_PATHS {
            let services = test_proxy_services();
            register(&services, &[HANDSHAKE]);
            let requests = on_requests(&services, |_| Forward);
            let responses = on_responses(&services, pass);

            let script = Script::new(vec![plugin_request(-100, HANDSHAKE, b"hello")], 1);
            let outcome = run(path, &services, script).await;

            assert_eq!(wire(&outcome.at_client), wire(&outcome.sent), "{path:?}");
            assert_eq!(outcome.at_client[0].payload[..5], var_int(-100)[..]);
            assert_eq!(
                wire(&outcome.at_backend),
                wire(&outcome.answered),
                "{path:?}"
            );
            assert_eq!(
                seen(&requests),
                vec![outcome.request(-100, HANDSHAKE, b"hello", true)],
                "{path:?}"
            );
            assert_eq!(
                seen(&responses),
                vec![outcome.response(-100, HANDSHAKE, true, b"from client")],
                "{path:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_replaced_request_and_response_keep_their_message_id() {
        for path in CLIENT_PATHS {
            let services = test_proxy_services();
            register(&services, &[DATA]);
            let requests = on_requests(&services, |_| Replace(Bytes::from_static(b"new data")));
            let responses = on_responses(&services, |_| LoginPluginResponseResult::Replace {
                successful: false,
                data: Bytes::from_static(b"rewritten"),
            });

            let script = Script::new(vec![plugin_request(-101, DATA, b"old data")], 1);
            let outcome = run(path, &services, script).await;

            let relayed = outcome.client_got(-101);
            assert_eq!(relayed.channel, DATA, "{path:?}");
            assert_eq!(relayed.data, b"new data", "{path:?}");
            assert_eq!(outcome.at_client[0].id, outcome.sent[0].id);
            assert_eq!(outcome.at_client[0].payload[..5], var_int(-101)[..]);
            let answer = outcome.backend_got(-101);
            assert!(!answer.successful, "{path:?}");
            assert_eq!(answer.data, b"rewritten", "{path:?}");
            assert_eq!(outcome.at_backend[0].id, outcome.answered[0].id);
            assert_eq!(outcome.at_backend[0].payload[..5], var_int(-101)[..]);
            assert_eq!(
                seen(&requests),
                vec![outcome.request(-101, DATA, b"old data", true)]
            );
            assert_eq!(
                seen(&responses),
                vec![outcome.response(-101, DATA, true, b"from client")]
            );
        }
    }

    #[tokio::test]
    async fn a_responded_request_never_reaches_the_client_and_the_login_completes() {
        for path in CLIENT_PATHS {
            let services = test_proxy_services();
            register(&services, &[HANDSHAKE, DATA]);
            let requests = on_requests(&services, |channel| {
                if channel == HANDSHAKE {
                    Respond(Some(Bytes::from_static(b"proxy")))
                } else {
                    Respond(None)
                }
            });
            let responses = on_responses(&services, pass);

            let script = Script::new(
                vec![
                    plugin_request(-100, HANDSHAKE, b"hello"),
                    plugin_request(-101, DATA, b"data"),
                ],
                0,
            );
            let outcome = run(path, &services, script).await;

            assert!(outcome.at_client.is_empty(), "{path:?}");
            let understood = outcome.backend_got(-100);
            assert!(understood.successful, "{path:?}");
            assert_eq!(understood.data, b"proxy", "{path:?}");
            assert!(not_understood(&outcome.backend_got(-101)), "{path:?}");
            assert_eq!(seen(&requests).len(), 2, "{path:?}");
            assert!(seen(&responses).is_empty(), "{path:?}");
        }
    }

    #[tokio::test]
    async fn an_unregistered_channel_fires_nothing_and_is_relayed_byte_for_byte() {
        for path in CLIENT_PATHS {
            let services = test_proxy_services();
            register(&services, &[HANDSHAKE]);
            let requests = on_requests(&services, |_| Respond(None));
            let responses = on_responses(&services, |_| LoginPluginResponseResult::Replace {
                successful: false,
                data: Bytes::new(),
            });

            let script = Script::new(vec![plugin_request(7, FORGE, b"mods")], 1);
            let outcome = run(path, &services, script).await;

            assert_eq!(wire(&outcome.at_client), wire(&outcome.sent), "{path:?}");
            assert_eq!(
                wire(&outcome.at_backend),
                wire(&outcome.answered),
                "{path:?}"
            );
            assert!(seen(&requests).is_empty(), "{path:?}");
            assert!(seen(&responses).is_empty(), "{path:?}");
        }
    }

    #[tokio::test]
    async fn a_response_listener_alone_still_sees_the_answers() {
        for path in CLIENT_PATHS {
            let services = test_proxy_services();
            register(&services, &[FORGE]);
            let responses = on_responses(&services, pass);

            let script = Script::new(vec![plugin_request(3, FORGE, b"mods")], 1);
            let outcome = run(path, &services, script).await;

            assert_eq!(wire(&outcome.at_client), wire(&outcome.sent), "{path:?}");
            assert_eq!(
                wire(&outcome.at_backend),
                wire(&outcome.answered),
                "{path:?}"
            );
            assert_eq!(
                seen(&responses),
                vec![outcome.response(3, FORGE, true, b"from client")],
                "{path:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_login_query_exchange_forwards_one_request_and_replaces_the_next() {
        for path in CLIENT_PATHS {
            let services = test_proxy_services();
            register(&services, &[HANDSHAKE, DATA]);
            let requests = on_requests(&services, |channel| {
                if channel == DATA {
                    Replace(Bytes::from_static(b"bridge address"))
                } else {
                    Forward
                }
            });
            let responses = on_responses(&services, pass);

            let script = Script {
                requests: vec![
                    plugin_request(-100, HANDSHAKE, b"hello"),
                    plugin_request(-101, DATA, b"backend address"),
                ],
                reaching_client: 2,
                answer: handshake_only_client,
            };
            let outcome = run(path, &services, script).await;

            assert_eq!(
                wire(&outcome.at_client[..1]),
                wire(&outcome.sent[..1]),
                "{path:?}"
            );
            assert_eq!(outcome.at_client[0].payload[..5], var_int(-100)[..]);
            assert_eq!(outcome.at_client[1].payload[..5], var_int(-101)[..]);
            assert_eq!(outcome.client_got(-101).data, b"bridge address", "{path:?}");
            assert_eq!(
                wire(&outcome.at_backend),
                wire(&outcome.answered),
                "{path:?}"
            );
            assert_eq!(
                seen(&requests),
                vec![
                    outcome.request(-100, HANDSHAKE, b"hello", true),
                    outcome.request(-101, DATA, b"backend address", true),
                ],
                "{path:?}"
            );
            assert_eq!(
                seen(&responses),
                vec![
                    outcome.response(-101, DATA, false, b""),
                    outcome.response(-100, HANDSHAKE, true, b"ready"),
                ],
                "{path:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_velocity_request_never_fires_even_on_a_registered_channel() {
        for path in CLIENT_PATHS {
            let services = velocity_services();
            register(&services, &[VELOCITY]);
            let requests = on_requests(&services, |_| Respond(None));
            let responses = on_responses(&services, pass);

            let reaching_client = match path {
                Path::Bridged => 0,
                Path::Relayed => 1,
            };
            let script = Script::new(vec![plugin_request(1, VELOCITY, &[4])], reaching_client);
            let outcome = run(path, &services, script).await;

            match path {
                Path::Bridged => {
                    let answer = outcome.backend_got(1);
                    assert!(answer.successful && !answer.data.is_empty());
                }
                Path::Relayed => {
                    assert_eq!(wire(&outcome.at_client), wire(&outcome.sent));
                    assert_eq!(wire(&outcome.at_backend), wire(&outcome.answered));
                }
            }
            assert!(seen(&requests).is_empty(), "{path:?}");
            assert!(seen(&responses).is_empty(), "{path:?}");
        }
    }

    #[tokio::test]
    async fn past_login_a_forwarded_or_replaced_request_is_answered_not_understood() {
        let services = test_proxy_services();
        register(&services, &[HANDSHAKE, DATA]);
        let requests = on_requests(&services, |channel| {
            if channel == DATA {
                Replace(Bytes::from_static(b"new data"))
            } else {
                Forward
            }
        });
        let responses = on_responses(&services, pass);

        let outcome = proxied(
            &services,
            vec![
                plugin_request(-100, HANDSHAKE, b"hello"),
                plugin_request(-101, DATA, b"data"),
            ],
        )
        .await;

        assert!(not_understood(&outcome.backend_got(-100)));
        assert!(not_understood(&outcome.backend_got(-101)));
        assert_eq!(outcome.at_backend[0].payload[..5], var_int(-100)[..]);
        assert_eq!(
            seen(&requests),
            vec![
                outcome.request(-100, HANDSHAKE, b"hello", false),
                outcome.request(-101, DATA, b"data", false),
            ]
        );
        assert!(seen(&responses).is_empty());
    }

    #[tokio::test]
    async fn past_login_a_plugin_answers_the_backend() {
        let services = test_proxy_services();
        register(&services, &[HANDSHAKE, DATA]);
        on_requests(&services, |channel| {
            if channel == HANDSHAKE {
                Respond(Some(Bytes::from_static(b"\x00")))
            } else {
                Respond(None)
            }
        });

        let outcome = proxied(
            &services,
            vec![
                plugin_request(-100, HANDSHAKE, b"hello"),
                plugin_request(-101, DATA, b"data"),
            ],
        )
        .await;

        let understood = outcome.backend_got(-100);
        assert!(understood.successful);
        assert_eq!(understood.data, b"\x00");
        assert_eq!(outcome.at_backend[0].payload[..5], var_int(-100)[..]);
        assert!(not_understood(&outcome.backend_got(-101)));
    }

    #[tokio::test]
    async fn past_login_unregistered_channels_and_velocity_fire_nothing() {
        let services = test_proxy_services();
        register(&services, &[VELOCITY, HANDSHAKE]);
        let requests = on_requests(&services, |_| Respond(Some(Bytes::from_static(b"never"))));

        let outcome = proxied(
            &services,
            vec![
                plugin_request(1, VELOCITY, &[4]),
                plugin_request(7, FORGE, b"mods"),
            ],
        )
        .await;

        assert!(not_understood(&outcome.backend_got(1)));
        assert!(not_understood(&outcome.backend_got(7)));
        assert_eq!(outcome.sent.len(), 2);
        assert!(seen(&requests).is_empty());
    }
}
