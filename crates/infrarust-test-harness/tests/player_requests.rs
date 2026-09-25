#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use infrarust_api::error::PlayerError;
use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::connection::ServerPreConnectEvent;
use infrarust_api::events::transfer::PreTransferEvent;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::player::{
    BossBar, ConnectionResult, MAX_COOKIE_SIZE, Player, ResourcePackRequest,
};
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::packets::cookie::CConfigStoreCookie;
use infrarust_protocol::packets::cookie::{
    CConfigCookieRequest, CCookieRequest, SConfigCookieResponse, SCookieResponse,
};
use infrarust_protocol::packets::play::chat::{CChatMessageLegacy, CSystemChatMessage};
use infrarust_protocol::packets::play::transfer::{CConfigTransfer, CTransfer};
use infrarust_protocol::packets::resource_pack::{CConfigResourcePackPush, ResourcePackResult};
use infrarust_test_harness::text::component_text;
use infrarust_test_harness::{
    ClientSession, CookieJar, DEFAULT_TIMEOUT, EventKind, FakeBackend, LoginOutcome, PacketFrame,
    ProtocolVersion, Recorder, ScriptedPlugin, ServerSpec, TestProxy, version_matrix, wire,
};

const T: Duration = DEFAULT_TIMEOUT;
const SEED_KEY: &str = "infrarust:seed";
const SEED: &[u8] = b"seeded";

fn system_text(frame: &PacketFrame, version: ProtocolVersion) -> Option<String> {
    if wire::is::<CSystemChatMessage>(frame, version) {
        let message = wire::decode::<CSystemChatMessage>(frame, version).ok()?;
        return Some(component_text(&message.content, version));
    }
    if wire::is::<CChatMessageLegacy>(frame, version) {
        let message = wire::decode::<CChatMessageLegacy>(frame, version).ok()?;
        return Some(component_text(message.content.as_bytes(), version));
    }
    None
}

async fn frames_until_marker(
    session: &mut ClientSession,
    player: &dyn Player,
    marker: &str,
) -> Vec<PacketFrame> {
    player.send_message(Component::text(marker)).unwrap();
    let version = session.version();
    let mut seen = Vec::new();
    loop {
        let frame = session.recv_frame(T).await.unwrap();
        if system_text(&frame, version).as_deref() == Some(marker) {
            return seen;
        }
        seen.push(frame);
    }
}

fn transfer_gate() -> ScriptedPlugin {
    ScriptedPlugin::new("transfer_gate").on::<PreTransferEvent>(EventPriority::NORMAL, |event| {
        match event.host.as_str() {
            "blocked.example.com" => event.deny(Component::text("No transfers there")),
            "old.example.com" => event.redirect("new.example.com", 25570),
            _ => {}
        }
    })
}

async fn transfer_proxy(recorder: &Recorder) -> (FakeBackend, TestProxy) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .plugin(transfer_gate())
        .start()
        .await
        .unwrap();
    (backend, proxy)
}

async fn join(proxy: &TestProxy, version: ProtocolVersion) -> (ClientSession, Arc<dyn Player>) {
    let session = proxy
        .client(version)
        .cookies(CookieJar::new().with(SEED_KEY, SEED))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    (session, player)
}

fn no_transfer(frames: &[PacketFrame], version: ProtocolVersion) {
    assert!(
        !frames
            .iter()
            .any(|frame| wire::is::<CTransfer>(frame, version)),
        "no transfer may reach the client"
    );
}

async fn plugin_transfers(version: ProtocolVersion) {
    let recorder = Recorder::new();
    let (_backend, proxy) = transfer_proxy(&recorder).await;
    let (mut session, player) = join(&proxy, version).await;

    player.transfer("play.example.com", 25566).await.unwrap();
    let sent = session.expect::<CTransfer>(T).await.unwrap();
    assert_eq!((sent.host.as_str(), sent.port), ("play.example.com", 25566));

    player.transfer("old.example.com", 25565).await.unwrap();
    let redirected = session.expect::<CTransfer>(T).await.unwrap();
    assert_eq!(
        (redirected.host.as_str(), redirected.port),
        ("new.example.com", 25570)
    );

    match player.transfer("blocked.example.com", 25565).await {
        Err(PlayerError::Denied(reason)) => assert_eq!(reason.to_plain(), "No transfers there"),
        other => panic!("expected the transfer to be denied, got {other:?}"),
    }
    no_transfer(
        &frames_until_marker(&mut session, player.as_ref(), "after the denial").await,
        version,
    );

    let events = recorder.of(EventKind::PreTransfer);
    let seen: Vec<(&str, &str)> = events
        .iter()
        .map(|e| {
            (
                e.detail["host"].as_str().unwrap(),
                e.detail["origin"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("play.example.com", "plugin"),
            ("old.example.com", "plugin"),
            ("blocked.example.com", "plugin"),
        ]
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(plugin_transfers; p766 = 766, p774 = 774);

async fn backend_transfers_are_intercepted(version: ProtocolVersion) {
    let recorder = Recorder::new();
    let (backend, proxy) = transfer_proxy(&recorder).await;
    let (mut session, player) = join(&proxy, version).await;
    let mut conn = backend.next_connection(T).await.unwrap();

    conn.send_packet(&CTransfer {
        host: "old.example.com".into(),
        port: 25565,
    })
    .await
    .unwrap();
    let redirected = session.expect::<CTransfer>(T).await.unwrap();
    assert_eq!(
        (redirected.host.as_str(), redirected.port),
        ("new.example.com", 25570)
    );

    conn.send_packet(&CTransfer {
        host: "blocked.example.com".into(),
        port: 25565,
    })
    .await
    .unwrap();
    conn.send_system_message_json(r#"{"text":"after the backend denial"}"#)
        .await
        .unwrap();
    let version_of = session.version();
    let mut seen = Vec::new();
    loop {
        let frame = session.recv_frame(T).await.unwrap();
        if system_text(&frame, version_of).as_deref() == Some("after the backend denial") {
            break;
        }
        seen.push(frame);
    }
    no_transfer(&seen, version);

    conn.send_packet(&CTransfer {
        host: "hub.example.com".into(),
        port: 25567,
    })
    .await
    .unwrap();
    let passed = session.expect::<CTransfer>(T).await.unwrap();
    assert_eq!(
        (passed.host.as_str(), passed.port),
        ("hub.example.com", 25567)
    );

    let origins: Vec<String> = recorder
        .of(EventKind::PreTransfer)
        .iter()
        .map(|e| e.detail["origin"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(origins, ["backend", "backend", "backend"]);
    drop(player);

    proxy.shutdown().await.unwrap();
}

version_matrix!(backend_transfers_are_intercepted; p766 = 766, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfers_and_cookies_need_1_20_5() {
    let recorder = Recorder::new();
    let (_backend, proxy) = transfer_proxy(&recorder).await;
    let (_session, player) = join(&proxy, ProtocolVersion::V1_20_3).await;

    assert!(matches!(
        player.transfer("play.example.com", 25565).await,
        Err(PlayerError::Unsupported(_))
    ));
    assert!(matches!(
        player.store_cookie(SEED_KEY, Bytes::from_static(b"x")),
        Err(PlayerError::Unsupported(_))
    ));
    assert!(matches!(
        player.request_cookie(SEED_KEY).await,
        Err(PlayerError::Unsupported(_))
    ));
    assert_eq!(recorder.count(EventKind::PreTransfer), 0);

    proxy.shutdown().await.unwrap();
}

async fn cookies_in_play(version: ProtocolVersion) {
    let recorder = Recorder::new();
    let (backend, proxy) = transfer_proxy(&recorder).await;
    let (session, player) = join(&proxy, version).await;
    let mut conn = backend.next_connection(T).await.unwrap();

    assert_eq!(
        player.request_cookie(SEED_KEY).await.unwrap(),
        Some(Bytes::from_static(SEED))
    );
    assert_eq!(
        player.request_cookie("infrarust:missing").await.unwrap(),
        None
    );

    player
        .store_cookie("session", Bytes::from_static(b"abc"))
        .unwrap();
    assert_eq!(
        player.request_cookie("minecraft:session").await.unwrap(),
        Some(Bytes::from_static(b"abc"))
    );
    assert_eq!(
        session.cookies().get("minecraft:session"),
        Some(b"abc".to_vec())
    );

    assert!(matches!(
        player.store_cookie("Bad Key", Bytes::new()),
        Err(PlayerError::InvalidArgument(_))
    ));
    assert!(matches!(
        player.store_cookie(SEED_KEY, Bytes::from(vec![0; MAX_COOKIE_SIZE + 1])),
        Err(PlayerError::InvalidArgument(_))
    ));

    conn.send_packet(&CCookieRequest {
        key: SEED_KEY.into(),
    })
    .await
    .unwrap();
    let answer = conn.expect::<SCookieResponse>(T).await.unwrap();
    assert_eq!(answer.key, SEED_KEY);
    assert_eq!(answer.payload.as_deref(), Some(SEED));

    session.chat("after the cookies").await.unwrap();
    let chats = conn.chat_until("after the cookies", T).await.unwrap();
    assert!(chats.is_empty(), "{chats:?}");
    let answered = conn
        .received()
        .iter()
        .filter(|frame| wire::is::<SCookieResponse>(frame, version))
        .count();
    assert_eq!(
        answered, 1,
        "the proxy's own cookie answers stay on the proxy"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(cookies_in_play; p766 = 766, p774 = 774);

async fn cookies_in_the_configuration_phase(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let jar = CookieJar::new().with(SEED_KEY, SEED);
    let client = proxy.client(version).cookies(jar.clone());
    let login = tokio::spawn(async move { client.login("Steve").await });
    let mut conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    assert_eq!(
        player.request_cookie(SEED_KEY).await.unwrap(),
        Some(Bytes::from_static(SEED))
    );
    player
        .store_cookie("infrarust:config", Bytes::from_static(b"cfg"))
        .unwrap();
    assert_eq!(
        player.request_cookie("infrarust:config").await.unwrap(),
        Some(Bytes::from_static(b"cfg"))
    );

    conn.send_packet(&CConfigCookieRequest {
        key: "infrarust:config".into(),
    })
    .await
    .unwrap();
    let answer = conn.expect::<SConfigCookieResponse>(T).await.unwrap();
    assert_eq!(answer.payload.as_deref(), Some(&b"cfg"[..]));

    conn.finish_config(T).await.unwrap();
    let outcome = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(outcome, LoginOutcome::Joined(_)));
    assert_eq!(jar.get("infrarust:config"), Some(b"cfg".to_vec()));
    let answered = conn
        .received()
        .iter()
        .filter(|frame| wire::is::<SConfigCookieResponse>(frame, version))
        .count();
    assert_eq!(answered, 1);

    proxy.shutdown().await.unwrap();
}

version_matrix!(cookies_in_the_configuration_phase; p766 = 766, p774 = 774);

fn deny_guarded() -> ScriptedPlugin {
    ScriptedPlugin::new("gate").on::<ServerPreConnectEvent>(EventPriority::NORMAL, |event| {
        if event.server.as_str() == "guarded" {
            event.deny(Component::text("Guarded"));
        }
    })
}

async fn connect_reports_the_outcome(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let backend_guarded = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("a")
                .backend(backend_a.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("b")
                .backend(backend_b.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("guarded")
                .backend(backend_guarded.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("down")
                .unreachable()
                .network("main")
                .patch(|table| {
                    table.insert(
                        "disconnect_message".into(),
                        toml::Value::String("Down for maintenance".into()),
                    );
                }),
        )
        .plugin(deny_guarded())
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client_for("a", version)
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    assert_eq!(
        player.connect(ServerId::new("a")).await.unwrap(),
        ConnectionResult::AlreadyConnected
    );
    assert_eq!(
        player.connect(ServerId::new("guarded")).await.unwrap(),
        ConnectionResult::Denied(Component::text("Guarded"))
    );
    match player.connect(ServerId::new("down")).await.unwrap() {
        ConnectionResult::Failed(reason) => assert_eq!(reason.to_plain(), "Down for maintenance"),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(player.current_server(), Some(ServerId::new("a")));

    assert_eq!(
        player.connect(ServerId::new("b")).await.unwrap(),
        ConnectionResult::Success
    );
    session.expect_join(T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("b")));
    backend_b.next_connection(T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

version_matrix!(connect_reports_the_outcome; p47 = 47, p764 = 764, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pending_connect_is_cancelled_when_the_player_is_kicked() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let held = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("a")
                .backend(backend.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("held")
                .backend(held.addr())
                .network("main"),
        )
        .start()
        .await
        .unwrap();
    let _session = proxy
        .client_for("a", ProtocolVersion(774))
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    let pending = {
        let player = Arc::clone(&player);
        tokio::spawn(async move { player.connect(ServerId::new("held")).await })
    };
    let _conn = held.next_connection(T).await.unwrap();
    tokio::time::timeout(T, player.disconnect(Component::text("bye")))
        .await
        .unwrap();

    let result = tokio::time::timeout(T, pending).await.unwrap().unwrap();
    assert_eq!(result.unwrap(), ConnectionResult::Cancelled);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarded_players_are_not_active() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(backend.addr()))
        .start()
        .await
        .unwrap();
    let _session = proxy
        .client(ProtocolVersion(774))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert!(!player.is_active());

    assert!(matches!(
        player.set_player_list_header_footer(Component::text("a"), Component::text("b")),
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.clear_title(true),
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.show_boss_bar(BossBar::new(Component::text("x"))),
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.send_resource_pack(ResourcePackRequest::new("https://example.com/p.zip")),
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.remove_resource_pack(None),
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.transfer("play.example.com", 25565).await,
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.store_cookie(SEED_KEY, Bytes::new()),
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.request_cookie(SEED_KEY).await,
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.connect(ServerId::new("pipe")).await,
        Err(PlayerError::NotActive)
    ));

    proxy.shutdown().await.unwrap();
}

struct Hold;

impl LimboHandler for Hold {
    fn name(&self) -> &str {
        "hold"
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async { HandlerResult::Hold })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn answers_given_in_limbo_reach_the_proxy() {
    let recorder = Recorder::new();
    let holder = ScriptedPlugin::new("holder").on_enable(|ctx| {
        ctx.register_limbo_handler(Box::new(Hold)).unwrap();
    });
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("hub")
                .unreachable()
                .limbo_handlers(["hold"]),
        )
        .plugin(holder)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let _session = proxy
        .client(ProtocolVersion(774))
        .cookies(CookieJar::new().with(SEED_KEY, SEED))
        .answer_resource_packs([
            ResourcePackResult::Accepted,
            ResourcePackResult::SuccessfullyLoaded,
        ])
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    recorder
        .wait_for_kind(EventKind::LimboEnter, T)
        .await
        .unwrap();

    let answer = tokio::time::timeout(T, player.request_cookie(SEED_KEY))
        .await
        .expect("a cookie asked for in limbo must be answered");
    assert_eq!(answer.unwrap(), Some(Bytes::from_static(SEED)));

    player
        .send_resource_pack(ResourcePackRequest::new("https://example.com/p.zip"))
        .unwrap();
    recorder
        .wait_for(
            |e| {
                e.kind == EventKind::PlayerResourcePackStatus
                    && e.detail["status"] == "successfully_loaded"
                    && e.detail["origin"] == "proxy"
            },
            T,
        )
        .await
        .unwrap();

    proxy.shutdown().await.unwrap();
}

async fn a_switch_configuration_phase_keeps_backend_requests(version: ProtocolVersion) {
    let recorder = Recorder::new();
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("a")
                .backend(backend_a.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("b")
                .backend(backend_b.addr())
                .network("main"),
        )
        .plugin(recorder.plugin())
        .plugin(transfer_gate())
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client_for("a", version)
        .unwrap()
        .cookies(CookieJar::new().with(SEED_KEY, SEED))
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    let switching = {
        let player = Arc::clone(&player);
        tokio::spawn(async move { player.connect(ServerId::new("b")).await })
    };
    let mut conn_b = backend_b.next_connection(T).await.unwrap();

    conn_b
        .send_packet(&CConfigCookieRequest {
            key: SEED_KEY.into(),
        })
        .await
        .unwrap();
    let answer = conn_b.expect::<SConfigCookieResponse>(T).await.unwrap();
    assert_eq!(answer.payload.as_deref(), Some(SEED));

    conn_b
        .send_packet(&CConfigTransfer {
            host: "old.example.com".into(),
            port: 25565,
        })
        .await
        .unwrap();
    let redirected = session.expect_config::<CConfigTransfer>(T).await.unwrap();
    assert_eq!(
        (redirected.host.as_str(), redirected.port),
        ("new.example.com", 25570)
    );

    conn_b.finish_config(T).await.unwrap();
    let result = tokio::time::timeout(T, switching).await.unwrap().unwrap();
    assert_eq!(result.unwrap(), ConnectionResult::Success);
    session.expect_join(T).await.unwrap();
    let transfer = recorder
        .wait_for_kind(EventKind::PreTransfer, T)
        .await
        .unwrap();
    assert_eq!(transfer.detail["origin"], "backend");

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_switch_configuration_phase_keeps_backend_requests; p766 = 766, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_backends_configuration_requests_are_not_replayed_to_limbo_players() {
    let version = ProtocolVersion(774);
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let holder = ScriptedPlugin::new("holder").on_enable(|ctx| {
        ctx.register_limbo_handler(Box::new(Hold)).unwrap();
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .server(
            ServerSpec::offline("hub")
                .unreachable()
                .limbo_handlers(["hold"]),
        )
        .plugin(holder)
        .start()
        .await
        .unwrap();

    let first = proxy.client_for("lobby", version).unwrap();
    let login = tokio::spawn(async move { first.login("Alex").await });
    let mut conn = backend.next_connection(T).await.unwrap();
    conn.send_packet(&CConfigCookieRequest {
        key: SEED_KEY.into(),
    })
    .await
    .unwrap();
    conn.send_packet(&CConfigStoreCookie {
        key: SEED_KEY.into(),
        payload: SEED.to_vec(),
    })
    .await
    .unwrap();
    conn.send_packet(&CConfigResourcePackPush {
        id: uuid::Uuid::from_u128(1),
        url: "https://example.com/p.zip".into(),
        hash: String::new(),
        forced: false,
        prompt: None,
    })
    .await
    .unwrap();
    conn.send_packet(&CConfigTransfer {
        host: "elsewhere.example.com".into(),
        port: 25565,
    })
    .await
    .unwrap();
    conn.finish_config(T).await.unwrap();
    let _first = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();

    let second = proxy
        .client_for("hub", version)
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let replayed: Vec<&str> = second
        .config_frames()
        .iter()
        .filter_map(|frame| {
            if wire::is::<CConfigTransfer>(frame, version) {
                Some("transfer")
            } else if wire::is::<CConfigCookieRequest>(frame, version) {
                Some("cookie request")
            } else if wire::is::<CConfigStoreCookie>(frame, version) {
                Some("store cookie")
            } else if wire::is::<CConfigResourcePackPush>(frame, version) {
                Some("resource pack")
            } else {
                None
            }
        })
        .collect();
    assert!(
        replayed.is_empty(),
        "another player's configuration requests reached a limbo player: {replayed:?}"
    );

    proxy.shutdown().await.unwrap();
}
