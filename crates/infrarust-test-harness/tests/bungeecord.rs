#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::connection::{ConnectCause, ServerPreConnectEvent};
use infrarust_api::events::messaging::PluginMessageEvent;
use infrarust_api::messaging::{ChannelId, Endpoint};
use infrarust_api::types::ServerId;
use toml::{Table, Value};

use infrarust_test_harness::plugin_message::{
    backend_message, bungee_channel, client_message, parse_register, register_channel,
    send_to_client, serverbound,
};
use infrarust_test_harness::{
    BackendConn, ClientSession, ConnectionState, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion,
    ScriptedPlugin, ServerSpec, TestProxy, TestProxyBuilder, version_matrix,
};

const T: Duration = DEFAULT_TIMEOUT;

fn utf(text: &str) -> Vec<u8> {
    let mut out = u16::try_from(text.len()).unwrap().to_be_bytes().to_vec();
    out.extend_from_slice(text.as_bytes());
    out
}

fn fields(parts: &[&str]) -> Vec<u8> {
    parts.iter().flat_map(|part| utf(part)).collect()
}

fn with_int(mut data: Vec<u8>, value: i32) -> Vec<u8> {
    data.extend_from_slice(&value.to_be_bytes());
    data
}

fn bungeecord(permissions: &[(&str, bool)]) -> impl FnOnce(&mut Table) + Send + 'static {
    let permissions: Vec<(String, bool)> = permissions
        .iter()
        .map(|(key, value)| ((*key).to_string(), *value))
        .collect();
    move |table: &mut Table| {
        let mut section = Table::new();
        section.insert("bungeecord".into(), Value::Boolean(true));
        let granted: Table = permissions
            .into_iter()
            .map(|(key, value)| (key, Value::Boolean(value)))
            .collect();
        section.insert("bungeecord_permissions".into(), Value::Table(granted));
        table.insert("plugin_messaging".into(), Value::Table(section));
    }
}

const EVERYTHING: &[(&str, bool)] = &[
    ("connect_other", true),
    ("message", true),
    ("message_raw", true),
    ("kick_player", true),
    ("kick_player_raw", true),
];

fn opted_in(spec: ServerSpec) -> ServerSpec {
    spec.patch(|table| {
        table.insert("bungeecord_channel".into(), Value::Boolean(true));
    })
}

async fn request(conn: &mut BackendConn, data: Vec<u8>) {
    let version = conn.version();
    send_to_client(conn, bungee_channel(version), data)
        .await
        .unwrap();
}

async fn response(conn: &mut BackendConn) -> Vec<u8> {
    let version = conn.version();
    backend_message(conn, bungee_channel(version), T)
        .await
        .unwrap()
        .data
}

async fn join(
    proxy: &TestProxy,
    server: &str,
    name: &str,
    version: ProtocolVersion,
) -> ClientSession {
    proxy
        .client_for(server, version)
        .unwrap()
        .login(name)
        .await
        .unwrap()
        .joined()
        .unwrap()
}

struct Network {
    lobby: FakeBackend,
    game: FakeBackend,
    other: FakeBackend,
}

impl Network {
    async fn spawn() -> Self {
        Self {
            lobby: FakeBackend::builder().spawn().await.unwrap(),
            game: FakeBackend::builder().spawn().await.unwrap(),
            other: FakeBackend::builder().spawn().await.unwrap(),
        }
    }

    fn proxy(&self, permissions: &[(&str, bool)]) -> TestProxyBuilder {
        TestProxy::builder()
            .server(opted_in(
                ServerSpec::offline("lobby")
                    .backend(self.lobby.addr())
                    .network("main"),
            ))
            .server(
                ServerSpec::offline("game")
                    .backend(self.game.addr())
                    .network("main"),
            )
            .server(
                ServerSpec::offline("other")
                    .backend(self.other.addr())
                    .network("other"),
            )
            .patch_config(bungeecord(permissions))
    }
}

async fn the_channel_is_off_by_default(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .server(opted_in(
            ServerSpec::offline("opted").backend(backend.addr()),
        ))
        .start()
        .await
        .unwrap();

    for server in ["lobby", "opted"] {
        let mut session = join(&proxy, server, server, version).await;
        let mut conn = backend.next_connection(T).await.unwrap();
        request(&mut conn, fields(&["GetServer"])).await;
        assert_eq!(
            client_message(&mut session, bungee_channel(version), T)
                .await
                .unwrap()
                .data,
            fields(&["GetServer"]),
            "{server}: the message reaches the client as before"
        );
        session.chat("marker").await.unwrap();
        conn.chat_until("marker", T).await.unwrap();
        let answered = conn.received().iter().any(|frame| {
            serverbound(frame, ConnectionState::Play, version)
                .unwrap()
                .is_some_and(|message| message.channel == bungee_channel(version))
        });
        assert!(!answered, "{server}: the proxy must not answer");
    }

    proxy.shutdown().await.unwrap();
}

version_matrix!(the_channel_is_off_by_default; p47 = 47, p774 = 774);

async fn requests_get_golden_responses(version: ProtocolVersion) {
    let network = Network::spawn().await;
    let proxy = network.proxy(&[]).start().await.unwrap();

    let _session = join(&proxy, "lobby", "Steve", version).await;
    let mut conn = network.lobby.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    let port = i32::from(player.remote_addr().port());
    let uuid = player.profile().uuid.simple().to_string();
    let game_port = network.game.addr().port();

    let registered = loop {
        let message = backend_message(&mut conn, register_channel(version), T)
            .await
            .unwrap();
        let channels = parse_register(&message.data);
        if channels.iter().any(|c| c == bungee_channel(version)) {
            break channels;
        }
    };
    assert!(registered.contains(&bungee_channel(version).to_string()));

    let cases: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (fields(&["GetServer"]), fields(&["GetServer", "lobby"])),
        (
            fields(&["GetServers"]),
            fields(&["GetServers", "game, lobby"]),
        ),
        (
            fields(&["IP"]),
            with_int(fields(&["IP", "127.0.0.1"]), port),
        ),
        (
            fields(&["IPOther", "steve"]),
            with_int(fields(&["IPOther", "Steve", "127.0.0.1"]), port),
        ),
        (
            fields(&["PlayerCount", "ALL"]),
            with_int(fields(&["PlayerCount", "ALL"]), 1),
        ),
        (
            fields(&["PlayerCount", "lobby"]),
            with_int(fields(&["PlayerCount", "lobby"]), 1),
        ),
        (
            fields(&["PlayerCount", "game"]),
            with_int(fields(&["PlayerCount", "game"]), 0),
        ),
        (
            fields(&["PlayerList", "ALL"]),
            fields(&["PlayerList", "ALL", "Steve"]),
        ),
        (
            fields(&["PlayerList", "game"]),
            fields(&["PlayerList", "game", ""]),
        ),
        (
            fields(&["GetPlayerServer", "Steve"]),
            fields(&["GetPlayerServer", "Steve", "lobby"]),
        ),
        (fields(&["UUID"]), fields(&["UUID", &uuid])),
        (
            fields(&["UUIDOther", "Steve"]),
            fields(&["UUIDOther", "Steve", &uuid]),
        ),
        (
            fields(&["ServerIP", "game"]),
            [
                fields(&["ServerIP", "game", "127.0.0.1"]),
                game_port.to_be_bytes().to_vec(),
            ]
            .concat(),
        ),
    ];
    for (asked, expected) in cases {
        request(&mut conn, asked.clone()).await;
        assert_eq!(response(&mut conn).await, expected, "request {asked:?}");
    }

    let literal = [
        0x00, 0x09, b'G', b'e', b't', b'S', b'e', b'r', b'v', b'e', b'r', 0x00, 0x05, b'l', b'o',
        b'b', b'b', b'y',
    ];
    request(&mut conn, fields(&["GetServer"])).await;
    assert_eq!(response(&mut conn).await, literal);

    proxy.shutdown().await.unwrap();
}

version_matrix!(requests_get_golden_responses; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

async fn requests_stay_in_the_network(version: ProtocolVersion) {
    let network = Network::spawn().await;
    let proxy = network.proxy(EVERYTHING).start().await.unwrap();

    let _steve = join(&proxy, "lobby", "Steve", version).await;
    let mut lobby = network.lobby.next_connection(T).await.unwrap();
    let alex = join(&proxy, "other", "Alex", version).await;
    let mut other = network.other.next_connection(T).await.unwrap();

    for refused in [
        fields(&["PlayerCount", "other"]),
        fields(&["PlayerList", "other"]),
        fields(&["ServerIP", "other"]),
        fields(&["UUIDOther", "Alex"]),
        fields(&["IPOther", "Alex"]),
        fields(&["GetPlayerServer", "Alex"]),
        fields(&["ConnectOther", "Alex", "game"]),
        fields(&["Connect", "other"]),
        fields(&["KickPlayer", "Alex", "gone"]),
        fields(&["Message", "Alex", "psst"]),
        [fields(&["Forward", "other", "chan"]), vec![0, 1, 7]].concat(),
        [fields(&["ForwardToPlayer", "Alex", "chan"]), vec![0, 1, 7]].concat(),
    ] {
        request(&mut lobby, refused).await;
    }
    request(&mut lobby, fields(&["PlayerList", "ALL"])).await;
    assert_eq!(
        response(&mut lobby).await,
        fields(&["PlayerList", "ALL", "Steve"])
    );
    request(&mut lobby, fields(&["PlayerCount", "ALL"])).await;
    assert_eq!(
        response(&mut lobby).await,
        with_int(fields(&["PlayerCount", "ALL"]), 1)
    );

    alex.chat("still here").await.unwrap();
    let seen = other.chat_until("still here", T).await.unwrap();
    let forwarded = other.received().iter().any(|frame| {
        serverbound(frame, ConnectionState::Play, version)
            .unwrap()
            .is_some_and(|message| message.channel == bungee_channel(version))
    });
    assert!(
        !forwarded,
        "nothing crossed into the other network: {seen:?}"
    );
    let steve = proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(steve.current_server(), Some(ServerId::new("lobby")));
    let alex_player = proxy.wait_for_player("Alex", T).await.unwrap();
    assert_eq!(alex_player.current_server(), Some(ServerId::new("other")));

    proxy.shutdown().await.unwrap();
}

version_matrix!(requests_stay_in_the_network; p47 = 47, p774 = 774);

async fn connect_goes_through_the_connection_events(version: ProtocolVersion) {
    let network = Network::spawn().await;
    let causes: Arc<Mutex<Vec<(String, ConnectCause)>>> = Arc::default();
    let seen = Arc::clone(&causes);
    let watcher = ScriptedPlugin::new("causes").on::<ServerPreConnectEvent>(
        EventPriority::NORMAL,
        move |event| {
            seen.lock()
                .unwrap()
                .push((event.player.profile().username.clone(), event.cause));
        },
    );
    let proxy = network
        .proxy(EVERYTHING)
        .plugin(watcher)
        .start()
        .await
        .unwrap();

    let mut steve = join(&proxy, "lobby", "Steve", version).await;
    let mut steve_lobby = network.lobby.next_connection(T).await.unwrap();
    let mut alex = join(&proxy, "lobby", "Alex", version).await;
    let _alex_lobby = network.lobby.next_connection(T).await.unwrap();

    request(&mut steve_lobby, fields(&["ConnectOther", "Alex", "game"])).await;
    alex.expect_join(T).await.unwrap();
    let alex_game = network.game.next_connection(T).await.unwrap();
    assert_eq!(alex_game.username(), "Alex");

    request(&mut steve_lobby, fields(&["Connect", "game"])).await;
    steve.expect_join(T).await.unwrap();
    let steve_game = network.game.next_connection(T).await.unwrap();
    assert_eq!(steve_game.username(), "Steve");

    let causes = causes.lock().unwrap().clone();
    assert!(
        causes.contains(&("Steve".to_string(), ConnectCause::PluginMessage)),
        "{causes:?}"
    );
    assert!(
        causes.contains(&("Alex".to_string(), ConnectCause::PluginMessage)),
        "{causes:?}"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(connect_goes_through_the_connection_events; p47 = 47, p764 = 764, p774 = 774);

async fn forward_reaches_a_backend_of_the_network(version: ProtocolVersion) {
    let network = Network::spawn().await;
    let proxy = network.proxy(&[]).start().await.unwrap();

    let _steve = join(&proxy, "lobby", "Steve", version).await;
    let mut lobby = network.lobby.next_connection(T).await.unwrap();
    let mut alex = join(&proxy, "game", "Alex", version).await;
    let mut game = network.game.next_connection(T).await.unwrap();
    proxy.wait_for_player("Alex", T).await.unwrap();

    let delivered = [utf("chan"), vec![0, 3, 1, 2, 3]].concat();
    request(
        &mut lobby,
        [fields(&["Forward", "game", "chan"]), vec![0, 3, 1, 2, 3]].concat(),
    )
    .await;
    assert_eq!(response(&mut game).await, delivered);

    request(
        &mut lobby,
        [fields(&["Forward", "ALL", "chan"]), vec![0, 3, 1, 2, 3]].concat(),
    )
    .await;
    assert_eq!(response(&mut game).await, delivered);

    request(
        &mut lobby,
        [
            fields(&["ForwardToPlayer", "Alex", "chan"]),
            vec![0, 3, 1, 2, 3],
        ]
        .concat(),
    )
    .await;
    assert_eq!(response(&mut game).await, delivered);

    request(&mut lobby, fields(&["GetServer"])).await;
    assert_eq!(response(&mut lobby).await, fields(&["GetServer", "lobby"]));

    request(&mut game, fields(&["GetServer"])).await;
    assert_eq!(
        client_message(&mut alex, bungee_channel(version), T)
            .await
            .unwrap()
            .data,
        fields(&["GetServer"]),
        "game did not opt in, so its request reaches the client"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(forward_reaches_a_backend_of_the_network; p47 = 47, p340 = 340, p764 = 764, p774 = 774);

async fn a_plugin_can_take_the_channel_over(version: ProtocolVersion) {
    let network = Network::spawn().await;
    let sources: Arc<Mutex<Vec<Endpoint>>> = Arc::default();
    let seen = Arc::clone(&sources);
    let takeover = ScriptedPlugin::new("takeover")
        .on_enable(|ctx| ctx.channel_registrar().register(ChannelId::bungeecord()))
        .on::<PluginMessageEvent>(EventPriority::NORMAL, move |event| {
            seen.lock().unwrap().push(event.source.clone());
            if event.data.as_ref() == fields(&["UUID"]).as_slice() {
                event.handled();
            }
        });
    let proxy = network.proxy(&[]).plugin(takeover).start().await.unwrap();

    let _steve = join(&proxy, "lobby", "Steve", version).await;
    let mut lobby = network.lobby.next_connection(T).await.unwrap();

    request(&mut lobby, fields(&["UUID"])).await;
    request(&mut lobby, fields(&["GetServer"])).await;
    assert_eq!(response(&mut lobby).await, fields(&["GetServer", "lobby"]));
    assert_eq!(
        sources.lock().unwrap().clone(),
        vec![
            Endpoint::Backend(ServerId::new("lobby")),
            Endpoint::Backend(ServerId::new("lobby"))
        ]
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_plugin_can_take_the_channel_over; p47 = 47, p774 = 774);

async fn write_subchannels_follow_the_permissions(version: ProtocolVersion) {
    let network = Network::spawn().await;
    let proxy = network.proxy(&[]).start().await.unwrap();
    let mut steve = join(&proxy, "lobby", "Steve", version).await;
    let mut lobby = network.lobby.next_connection(T).await.unwrap();

    request(&mut lobby, fields(&["Message", "Steve", "hidden"])).await;
    request(&mut lobby, fields(&["KickPlayer", "Steve", "gone"])).await;
    request(&mut lobby, fields(&["ConnectOther", "Steve", "game"])).await;
    request(&mut lobby, fields(&["GetServer"])).await;
    assert_eq!(response(&mut lobby).await, fields(&["GetServer", "lobby"]));
    steve.chat("still here").await.unwrap();
    lobby.chat_until("still here", T).await.unwrap();
    proxy.shutdown().await.unwrap();
    drop(steve);

    let network = Network::spawn().await;
    let proxy = network.proxy(EVERYTHING).start().await.unwrap();
    steve = join(&proxy, "lobby", "Steve", version).await;
    let mut lobby = network.lobby.next_connection(T).await.unwrap();

    request(&mut lobby, fields(&["Message", "Steve", "\u{a7}ahello"])).await;
    assert_eq!(steve.expect_system_text(T).await.unwrap(), "hello");
    request(
        &mut lobby,
        fields(&["MessageRaw", "ALL", r#"{"text":"to everyone"}"#]),
    )
    .await;
    assert_eq!(steve.expect_system_text(T).await.unwrap(), "to everyone");
    request(&mut lobby, fields(&["KickPlayer", "Steve", "\u{a7}cbye"])).await;
    let info = steve.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, "bye");

    proxy.shutdown().await.unwrap();
}

version_matrix!(write_subchannels_follow_the_permissions; p47 = 47, p774 = 774);
