#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use infrarust_api::event::EventPriority;
use infrarust_api::events::messaging::PluginMessageEvent;
use infrarust_api::messaging::{
    ChannelId, Endpoint, MessagePhase, MessagingError, ServerMessenger,
};
use infrarust_api::types::ServerId;
use infrarust_protocol::packets::play::plugin_message::{CPluginMessage, SPluginMessage};
use tokio::sync::mpsc;

use infrarust_test_harness::plugin_message::{
    BUNGEE, LEGACY_BUNGEE, PluginMessage, backend_message, client_message, clientbound,
    config_messages, send_to_backend, send_to_client, serverbound, to_client,
};
use infrarust_test_harness::wire;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin, ServerSpec,
    TestProxy, version_matrix,
};

const T: Duration = DEFAULT_TIMEOUT;

fn connect_request(server: &str) -> Vec<u8> {
    let mut data = Vec::new();
    for field in ["Connect", server] {
        data.extend_from_slice(&u16::try_from(field.len()).unwrap().to_be_bytes());
        data.extend_from_slice(field.as_bytes());
    }
    data
}

async fn a_client_cannot_spoof_backend_plugin_channels(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(version)
        .login("Mallory")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    let spoofed = [LEGACY_BUNGEE, BUNGEE, "velocity:player_info"];
    for channel in spoofed {
        send_to_backend(&session, channel, connect_request("admin"))
            .await
            .unwrap();
    }
    send_to_backend(&session, "mod:hello", b"still forwarded".to_vec())
        .await
        .unwrap();
    session.chat("marker").await.unwrap();
    conn.chat_until("marker", T).await.unwrap();

    let reached: Vec<String> = conn
        .received()
        .iter()
        .filter(|frame| wire::is::<SPluginMessage>(frame, version))
        .filter_map(|frame| serverbound(frame, ConnectionState::Play, version).unwrap())
        .map(|message| message.channel)
        .collect();
    assert_eq!(reached, vec!["mod:hello".to_string()]);

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_client_cannot_spoof_backend_plugin_channels; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

const ECHO: &str = "test:echo";

#[derive(Debug, Clone)]
struct Observed {
    source: Endpoint,
    channel: ChannelId,
    raw_channel: String,
    data: Bytes,
    phase: MessagePhase,
}

fn echo_plugin(tx: mpsc::UnboundedSender<Observed>) -> ScriptedPlugin {
    ScriptedPlugin::new("echo")
        .on_enable(|ctx| {
            ctx.channel_registrar()
                .register(ChannelId::pair(ECHO, "TestEcho").unwrap());
        })
        .on::<PluginMessageEvent>(EventPriority::NORMAL, move |event| {
            let _ = tx.send(Observed {
                source: event.source.clone(),
                channel: event.channel.clone(),
                raw_channel: event.raw_channel.clone(),
                data: event.data.clone(),
                phase: event.phase,
            });
            match event.data.as_ref() {
                b"drop" => event.handled(),
                b"swap" => event.replace(Bytes::from_static(b"swapped")),
                _ => {}
            }
        })
}

fn echo_name(version: ProtocolVersion) -> &'static str {
    if version.no_less_than(ProtocolVersion::V1_13) {
        ECHO
    } else {
        "TestEcho"
    }
}

async fn observed(rx: &mut mpsc::UnboundedReceiver<Observed>) -> Observed {
    tokio::time::timeout(T, rx.recv())
        .await
        .expect("a plugin message event in time")
        .expect("the plugin is alive")
}

fn plugin_messages(
    frames: &[infrarust_test_harness::PacketFrame],
    version: ProtocolVersion,
) -> Vec<PluginMessage> {
    frames
        .iter()
        .filter_map(|frame| serverbound(frame, ConnectionState::Play, version).unwrap())
        .collect()
}

async fn registered_channels_reach_plugins(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(echo_plugin(tx))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    let name = echo_name(version);

    send_to_backend(&session, name, b"hello".to_vec())
        .await
        .unwrap();
    let event = observed(&mut rx).await;
    assert_eq!(event.source, Endpoint::Client);
    assert_eq!(event.channel, ChannelId::pair(ECHO, "TestEcho").unwrap());
    assert_eq!(event.raw_channel, name);
    assert_eq!(event.data.as_ref(), b"hello");
    assert_eq!(event.phase, MessagePhase::Play);
    assert_eq!(
        backend_message(&mut conn, name, T).await.unwrap().data,
        b"hello"
    );

    send_to_backend(&session, name, b"drop".to_vec())
        .await
        .unwrap();
    send_to_backend(&session, name, b"swap".to_vec())
        .await
        .unwrap();
    session.chat("marker").await.unwrap();
    conn.chat_until("marker", T).await.unwrap();
    let echoed: Vec<Vec<u8>> = plugin_messages(&conn.received(), version)
        .into_iter()
        .filter(|message| message.channel == name)
        .map(|message| message.data)
        .collect();
    assert_eq!(echoed, vec![b"hello".to_vec(), b"swapped".to_vec()]);

    send_to_client(&mut conn, name, b"from backend".to_vec())
        .await
        .unwrap();
    let event = loop {
        let event = observed(&mut rx).await;
        if event.source != Endpoint::Client {
            break event;
        }
    };
    assert_eq!(event.source, Endpoint::Backend(ServerId::new("lobby")));
    assert_eq!(event.data.as_ref(), b"from backend");
    assert_eq!(
        client_message(&mut session, name, T).await.unwrap().data,
        b"from backend"
    );

    send_to_client(&mut conn, name, b"drop".to_vec())
        .await
        .unwrap();
    send_to_client(&mut conn, name, b"swap".to_vec())
        .await
        .unwrap();
    assert_eq!(
        client_message(&mut session, name, T).await.unwrap().data,
        b"swapped"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(registered_channels_reach_plugins; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

async fn unregistered_channels_pass_byte_identical(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(echo_plugin(tx))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    let sent = send_to_backend(&session, "free:channel", vec![0, 1, 2, 0xFF])
        .await
        .unwrap();
    session.chat("marker").await.unwrap();
    conn.chat_until("marker", T).await.unwrap();
    assert!(
        conn.received()
            .iter()
            .any(|frame| frame.id == sent.id && frame.payload == sent.payload)
    );

    let frame = to_client(
        &PluginMessage::new("free:channel", vec![9, 8, 7]),
        ConnectionState::Play,
        version,
    )
    .unwrap();
    conn.send_frame(&frame).await.unwrap();
    let got = loop {
        let got = session.recv_frame(T).await.unwrap();
        if wire::is::<CPluginMessage>(&got, version)
            && clientbound(&got, ConnectionState::Play, version)
                .unwrap()
                .is_some_and(|message| message.channel == "free:channel")
        {
            break got;
        }
    };
    assert_eq!((got.id, &got.payload), (frame.id, &frame.payload));
    assert!(
        rx.try_recv().is_err(),
        "no event for an unregistered channel"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(unregistered_channels_pass_byte_identical; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

async fn plugins_send_in_play(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    let channel = ChannelId::pair(ECHO, "TestEcho").unwrap();

    player
        .send_plugin_message(&channel, Bytes::from_static(b"to client"))
        .unwrap();
    assert_eq!(
        client_message(&mut session, echo_name(version), T)
            .await
            .unwrap()
            .data,
        b"to client"
    );
    player
        .send_plugin_message_to_backend(&channel, Bytes::from_static(b"to backend"))
        .unwrap();
    assert_eq!(
        backend_message(&mut conn, echo_name(version), T)
            .await
            .unwrap()
            .data,
        b"to backend"
    );

    let too_big = Bytes::from(vec![0; 32_768]);
    assert!(
        player
            .send_plugin_message_to_backend(&channel, too_big)
            .is_err()
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(plugins_send_in_play; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

async fn plugins_send_in_the_configuration_phase(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(echo_plugin(tx))
        .start()
        .await
        .unwrap();

    let client = proxy.client(version);
    let login = tokio::spawn(async move { client.login("Steve").await });
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.state(), ConnectionState::Config);
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    let channel = ChannelId::modern(ECHO).unwrap();

    player
        .send_plugin_message_to_backend(&channel, Bytes::from_static(b"config backend"))
        .unwrap();
    let message = backend_message(&mut conn, ECHO, T).await.unwrap();
    assert_eq!(message.data, b"config backend");

    player
        .send_plugin_message(&channel, Bytes::from_static(b"config client"))
        .unwrap();
    send_to_client(&mut conn, ECHO, b"config from backend".to_vec())
        .await
        .unwrap();
    let event = observed(&mut rx).await;
    assert_eq!(event.phase, MessagePhase::Configuration);
    assert_eq!(event.source, Endpoint::Backend(ServerId::new("lobby")));

    conn.finish_config(T).await.unwrap();
    let session = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();
    let data: Vec<Vec<u8>> = config_messages(&session, ECHO)
        .unwrap()
        .into_iter()
        .map(|message| message.data)
        .collect();
    assert_eq!(
        data,
        vec![b"config client".to_vec(), b"config from backend".to_vec()]
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(plugins_send_in_the_configuration_phase; p764 = 764, p774 = 774);

async fn the_server_messenger_needs_a_carrier(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let messenger: Arc<Mutex<Option<Arc<dyn ServerMessenger>>>> = Arc::default();
    let slot = Arc::clone(&messenger);
    let plugin = ScriptedPlugin::new("messenger").on_enable(move |ctx| {
        *slot.lock().unwrap() = Some(ctx.server_messenger());
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .start()
        .await
        .unwrap();
    let messenger = messenger.lock().unwrap().clone().unwrap();
    let lobby = ServerId::new("lobby");
    let channel = ChannelId::modern(ECHO).unwrap();

    assert_eq!(
        messenger.send_to_server(&lobby, &channel, Bytes::from_static(b"nobody")),
        Err(MessagingError::NoCarrier)
    );

    let _session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();

    assert_eq!(
        messenger.send_to_server(&lobby, &channel, Bytes::from_static(b"carried")),
        Ok(1)
    );
    assert_eq!(
        backend_message(&mut conn, ECHO, T).await.unwrap().data,
        b"carried"
    );
    assert_eq!(
        messenger.send_to_server(&ServerId::new("elsewhere"), &channel, Bytes::new()),
        Err(MessagingError::NoCarrier)
    );
    assert!(matches!(
        messenger.send_to_server(&lobby, &channel, Bytes::from(vec![0; 40_000])),
        Err(MessagingError::TooLarge { .. })
    ));

    proxy.shutdown().await.unwrap();
}

version_matrix!(the_server_messenger_needs_a_carrier; p47 = 47, p774 = 774);
