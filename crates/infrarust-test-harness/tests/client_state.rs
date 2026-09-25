#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use bytes::Bytes;
use infrarust_api::error::PlayerError;
use infrarust_api::event::{BoxFuture, EventPriority, PacketDirection};
use infrarust_api::events::client::{
    PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerSettingsChangedEvent,
};
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::messaging::ChannelId;
use infrarust_api::player::{ChatMode, ClientSettings, MainHand, ParticleStatus};
use infrarust_api::types::ServerId;
use infrarust_protocol::packets::play::client_information::ClientInformation;
use infrarust_protocol::packets::play::keepalive::{CKeepAlive, SKeepAlive};
use tokio::sync::mpsc;

use infrarust_test_harness::plugin_message::{
    ClientHello, PluginMessage, as_seen_by, expect_client_state, information_frame,
    register_channel, register_data, send_to_client, to_backend,
};
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin, ServerSpec,
    TestProxy, version_matrix,
};

const T: Duration = DEFAULT_TIMEOUT;

async fn a_switch_replays_the_client_state(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
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
        .start()
        .await
        .unwrap();

    let hello = ClientHello::sample();
    let expected = as_seen_by(hello.information.as_ref().unwrap(), version).unwrap();
    let mut session = proxy
        .client(version)
        .hello(hello.clone())
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn_a = backend_a.next_connection(T).await.unwrap();
    let on_a = expect_client_state(&mut conn_a, &hello, T).await.unwrap();
    assert_eq!(on_a.information.as_ref(), Some(&expected));

    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    player.switch_server(ServerId::new("b")).await.unwrap();
    session.expect_join(T).await.unwrap();

    let mut conn_b = backend_b.next_connection(T).await.unwrap();
    let on_b = expect_client_state(&mut conn_b, &hello, T).await.unwrap();
    assert_eq!(on_b.information, Some(expected));
    assert_eq!(on_b.brand, hello.brand);
    assert_eq!(on_b.channels, hello.channels);

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_switch_replays_the_client_state; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

struct Pass;

impl LimboHandler for Pass {
    fn name(&self) -> &str {
        "pass"
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async { HandlerResult::Accept })
    }
}

async fn a_limbo_gate_keeps_the_client_state(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let gate = ScriptedPlugin::new("gate").on_enable(|ctx| {
        ctx.register_limbo_handler(Box::new(Pass)).unwrap();
    });
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .limbo_handlers(["pass"]),
        )
        .plugin(gate)
        .start()
        .await
        .unwrap();

    let hello = ClientHello::sample();
    let expected = as_seen_by(hello.information.as_ref().unwrap(), version).unwrap();
    let _session = proxy
        .client(version)
        .hello(hello.clone())
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();

    let mut conn = backend.next_connection(T).await.unwrap();
    let seen = expect_client_state(&mut conn, &hello, T).await.unwrap();
    assert_eq!(seen.information, Some(expected));
    assert_eq!(seen.brand, hello.brand);
    assert_eq!(seen.channels, hello.channels);

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_limbo_gate_keeps_the_client_state; p764 = 764, p774 = 774);

fn expected_settings(information: &ClientInformation) -> ClientSettings {
    let mut settings = ClientSettings::new(information.locale.clone());
    settings.view_distance = u8::try_from(information.view_distance).unwrap();
    settings.chat_mode = ChatMode::from_id(information.chat_mode);
    settings.chat_colors = information.chat_colors;
    settings.skin_parts = infrarust_api::player::SkinParts::new(information.displayed_skin_parts);
    settings.main_hand = MainHand::from_id(information.main_hand);
    settings.text_filtering = information.text_filtering;
    settings.allow_listing = information.allow_server_listings;
    settings.particle_status = ParticleStatus::from_id(information.particle_status);
    settings
}

async fn the_player_reports_its_client_state(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let hello = ClientHello::sample();
    let seen = as_seen_by(hello.information.as_ref().unwrap(), version).unwrap();
    let _session = proxy
        .client(version)
        .hello(hello.clone())
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    expect_client_state(&mut conn, &hello, T).await.unwrap();

    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.client_brand().as_deref(), Some("fabric"));
    let settings = player.settings().unwrap();
    assert_eq!(settings, expected_settings(&seen));
    assert_eq!(settings.locale, "fr_fr");
    assert_eq!(settings.view_distance, 7);
    assert_eq!(settings.chat_mode, ChatMode::CommandsOnly);
    assert!(settings.skin_parts.cape() && !settings.skin_parts.jacket());
    assert!(settings.skin_parts.hat() && !settings.skin_parts.right_pants());
    assert_eq!(player.known_channels(), hello.channels);
    assert_eq!(
        player.virtual_host(),
        proxy.domain("lobby").map(str::to_string)
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(the_player_reports_its_client_state; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

async fn ping_is_measured_from_keepalives(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let _session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.ping(), None);

    conn.send_packet(&CKeepAlive { id: 1234 }).await.unwrap();
    assert_eq!(conn.expect::<SKeepAlive>(T).await.unwrap().id, 1234);
    let ping = player.ping().expect("the answered keepalive gives a ping");
    assert!(ping < T, "{ping:?}");

    conn.send_packet(&CKeepAlive { id: 99 }).await.unwrap();
    conn.expect::<SKeepAlive>(T).await.unwrap();
    assert!(player.ping().is_some());

    proxy.shutdown().await.unwrap();
}

version_matrix!(ping_is_measured_from_keepalives; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

enum Seen {
    Brand(String),
    Settings(ClientSettings),
    Channels(Vec<String>, PacketDirection),
}

fn watcher(tx: mpsc::UnboundedSender<Seen>) -> ScriptedPlugin {
    let brand = tx.clone();
    let settings = tx.clone();
    ScriptedPlugin::new("watcher")
        .on::<PlayerClientBrandEvent>(EventPriority::NORMAL, move |event| {
            let _ = brand.send(Seen::Brand(event.brand.clone()));
        })
        .on::<PlayerSettingsChangedEvent>(EventPriority::NORMAL, move |event| {
            let _ = settings.send(Seen::Settings(event.settings.clone()));
        })
        .on::<PlayerChannelRegisterEvent>(EventPriority::NORMAL, move |event| {
            let _ = tx.send(Seen::Channels(event.channels.clone(), event.direction));
        })
}

async fn next(rx: &mut mpsc::UnboundedReceiver<Seen>) -> Seen {
    tokio::time::timeout(T, rx.recv())
        .await
        .expect("an event in time")
        .expect("the watcher is alive")
}

async fn client_events_carry_the_client_state(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(watcher(tx))
        .start()
        .await
        .unwrap();

    let hello = ClientHello::sample();
    let seen = as_seen_by(hello.information.as_ref().unwrap(), version).unwrap();
    let session = proxy
        .client(version)
        .hello(hello.clone())
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    let mut brand = None;
    let mut settings = None;
    let mut channels = None;
    for _ in 0..3 {
        match next(&mut rx).await {
            Seen::Brand(value) => brand = Some(value),
            Seen::Settings(value) => settings = Some(value),
            Seen::Channels(value, direction) => channels = Some((value, direction)),
        }
    }
    assert_eq!(brand.as_deref(), Some("fabric"));
    assert_eq!(settings, Some(expected_settings(&seen)));
    assert_eq!(
        channels,
        Some((hello.channels.clone(), PacketDirection::Serverbound))
    );

    let mut changed = seen.clone();
    changed.view_distance = 12;
    session
        .send_frame(&information_frame(&changed, ConnectionState::Play, version).unwrap())
        .await
        .unwrap();
    match next(&mut rx).await {
        Seen::Settings(settings) => assert_eq!(settings.view_distance, 12),
        _ => panic!("expected the changed settings"),
    }
    session
        .send_frame(&information_frame(&changed, ConnectionState::Play, version).unwrap())
        .await
        .unwrap();

    send_to_client(
        &mut conn,
        register_channel(version),
        register_data(&["server:side"]),
    )
    .await
    .unwrap();
    match next(&mut rx).await {
        Seen::Channels(channels, direction) => {
            assert_eq!(channels, vec!["server:side".to_string()]);
            assert_eq!(direction, PacketDirection::Clientbound);
        }
        _ => panic!("expected the backend's registration, not a repeat of unchanged settings"),
    }

    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.known_channels(), hello.channels);
    let unregister = PluginMessage::new(
        infrarust_test_harness::plugin_message::unregister_channel(version),
        register_data(&["mod:sync"]),
    );
    session
        .send_frame(&to_backend(&unregister, ConnectionState::Play, version).unwrap())
        .await
        .unwrap();
    session.chat("sync").await.unwrap();
    conn.chat_until("sync", T).await.unwrap();
    assert_eq!(player.known_channels(), vec!["infrarust:test".to_string()]);

    proxy.shutdown().await.unwrap();
}

version_matrix!(client_events_carry_the_client_state; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

async fn forwarded_players_have_no_plugin_messaging(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let _session = proxy
        .client(version)
        .hello(ClientHello::sample())
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    let channel = ChannelId::modern("test:echo").unwrap();
    assert!(matches!(
        player.send_plugin_message(&channel, Bytes::from_static(b"x")),
        Err(PlayerError::NotActive)
    ));
    assert!(matches!(
        player.send_plugin_message_to_backend(&channel, Bytes::from_static(b"x")),
        Err(PlayerError::NotActive)
    ));
    assert_eq!(player.client_brand(), None);
    assert_eq!(player.settings(), None);
    assert!(player.known_channels().is_empty());
    assert_eq!(player.ping(), None);
    assert_eq!(
        player.virtual_host(),
        proxy.domain("lobby").map(str::to_string)
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(forwarded_players_have_no_plugin_messaging; p47 = 47, p774 = 774);

struct Hold {
    chats: mpsc::UnboundedSender<String>,
}

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

    fn on_chat<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
        message: &'a str,
    ) -> BoxFuture<'a, ()> {
        let _ = self.chats.send(message.to_string());
        Box::pin(async {})
    }
}

async fn ping_is_measured_in_limbo(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (tx, mut chats) = mpsc::unbounded_channel();
    let holder = ScriptedPlugin::new("holder").on_enable(move |ctx| {
        ctx.register_limbo_handler(Box::new(Hold { chats: tx.clone() }))
            .unwrap();
    });
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .limbo_handlers(["hold"]),
        )
        .plugin(holder)
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .hello(ClientHello::sample())
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    let channel = ChannelId::modern("test:round").unwrap();

    let rounds = async {
        let mut round = 0_u32;
        while player.ping().is_none() {
            round += 1;
            player
                .send_plugin_message(&channel, Bytes::from(format!("round {round}")))
                .unwrap();
            let echoed = infrarust_test_harness::plugin_message::client_message(
                &mut session,
                "test:round",
                T,
            )
            .await
            .unwrap();
            assert_eq!(echoed.data, format!("round {round}").into_bytes());
            session.chat("sync").await.unwrap();
            assert_eq!(chats.recv().await.unwrap(), "sync");
        }
    };
    tokio::time::timeout(T, rounds)
        .await
        .expect("the limbo keepalive is answered");
    let ping = player.ping().unwrap();
    assert!(ping < T, "{ping:?}");
    assert_eq!(player.client_brand().as_deref(), Some("fabric"));
    assert_eq!(player.settings().map(|s| s.view_distance), Some(7));
    assert_eq!(backend.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

version_matrix!(ping_is_measured_in_limbo; p47 = 47, p764 = 764, p774 = 774);
