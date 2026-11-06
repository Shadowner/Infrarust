#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::event::BoxFuture;
use infrarust_api::player::{ConnectionResult, Player};
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::packets::config::CRegistryData;
use infrarust_protocol::packets::play::chat::CSystemChatMessage;
use infrarust_protocol::packets::play::commands::{CCommands, CommandNode, MAX_COMMAND_NODES};
use infrarust_test_harness::text::component_text;
use infrarust_test_harness::{
    ClientSession, DEFAULT_TIMEOUT, FakeBackend, PacketFrame, ProtocolVersion, ScriptedPlugin,
    ServerSpec, TestProxy, holding_handler, version_matrix, wire,
};

const T: Duration = DEFAULT_TIMEOUT;

const HOLD: &str = "hostile_backend_hold";
const REGISTRY_MARKER: &[u8] = b"hostile registry";

async fn a_backends_registry_never_reaches_the_next_limbo_player(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let (handler, _holds) = holding_handler(HOLD);
    let handler = Mutex::new(Some(handler));
    let holder = ScriptedPlugin::new("holder").on_enable(move |ctx| {
        let handler = handler.lock().unwrap().take().unwrap();
        ctx.register_limbo_handler(handler).unwrap();
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .server(
            ServerSpec::offline("hub")
                .unreachable()
                .limbo_handlers([HOLD]),
        )
        .plugin(holder)
        .start()
        .await
        .unwrap();

    let first = proxy.client_for("lobby", version).unwrap();
    let login = tokio::spawn(async move { first.login("Alex").await });
    let mut conn = backend.next_connection(T).await.unwrap();
    conn.send_packet(&CRegistryData {
        data: REGISTRY_MARKER.to_vec(),
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
    let frames = second.config_frames();
    assert!(!frames.is_empty(), "the limbo player got no registry data");
    assert!(
        frames
            .iter()
            .all(|frame| frame.payload.as_ref() != REGISTRY_MARKER),
        "a frame chosen by the backend reached another player"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_backends_registry_never_reaches_the_next_limbo_player; p764 = 764, p766 = 766, p777 = 777);

async fn join(proxy: &TestProxy, version: ProtocolVersion) -> (ClientSession, Arc<dyn Player>) {
    let session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    (session, player)
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
        if wire::is::<CSystemChatMessage>(&frame, version) {
            let message = wire::decode::<CSystemChatMessage>(&frame, version).unwrap();
            if component_text(&message.content, version) == marker {
                return seen;
            }
        }
        seen.push(frame);
    }
}

struct Noop;

impl CommandHandler for Noop {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

const TREE_BYTE_LIMIT: usize = 1024 * 1024;
const REFRESH_SETTLE: Duration = Duration::from_millis(200);

fn wide_tree(nodes: usize, name_len: usize) -> CCommands {
    let mut tree = vec![CommandNode::root()];
    for i in 1..nodes {
        tree.push(CommandNode::literal_executable(&format!("{i:0>name_len$}")));
        tree[0].children.push(i32::try_from(i).unwrap());
    }
    CCommands {
        nodes: tree,
        root_index: 0,
    }
}

async fn next_tree(session: &mut ClientSession) -> PacketFrame {
    let version = session.version();
    loop {
        let frame = session.recv_frame(T).await.unwrap();
        if wire::is::<CCommands>(&frame, version) {
            return frame;
        }
    }
}

async fn relayed_as_is_and_never_refreshed(version: ProtocolVersion, hostile: PacketFrame) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(ScriptedPlugin::new("tester"))
        .start()
        .await
        .unwrap();
    let context = proxy.plugin_context("tester").await.unwrap();
    let commands = context.command_manager();
    commands
        .register(CommandSpec::new("open"), Box::new(Noop))
        .unwrap();
    let (mut session, player) = join(&proxy, version).await;
    let mut conn = backend.next_connection(T).await.unwrap();

    let small = wire::encode(&wide_tree(2, 4), version).unwrap();
    conn.send_frame(&small).await.unwrap();
    let injected = next_tree(&mut session).await;
    assert!(
        injected.payload != small.payload,
        "an ordinary tree gets the proxy's commands"
    );

    conn.send_frame(&hostile).await.unwrap();
    let relayed = next_tree(&mut session).await;
    assert!(
        relayed.payload == hostile.payload,
        "the tree must reach the client byte for byte ({} sent, {} received)",
        hostile.payload.len(),
        relayed.payload.len()
    );

    commands
        .register(CommandSpec::new("late"), Box::new(Noop))
        .unwrap();
    tokio::time::sleep(REFRESH_SETTLE).await;
    let after = frames_until_marker(&mut session, player.as_ref(), "after").await;
    assert!(
        !after
            .iter()
            .any(|frame| wire::is::<CCommands>(frame, version)),
        "a stale tree was sent to the client"
    );

    proxy.shutdown().await.unwrap();
}

async fn too_many_nodes_are_relayed_untouched(version: ProtocolVersion) {
    let hostile = wire::encode(&wide_tree(MAX_COMMAND_NODES + 1, 1), version).unwrap();
    relayed_as_is_and_never_refreshed(version, hostile).await;
}

version_matrix!(too_many_nodes_are_relayed_untouched; p764 = 764, p777 = 777);

async fn too_many_bytes_are_relayed_untouched(version: ProtocolVersion) {
    let hostile = wire::encode(&wide_tree(20_000, 60), version).unwrap();
    assert!(hostile.payload.len() > TREE_BYTE_LIMIT);
    relayed_as_is_and_never_refreshed(version, hostile).await;
}

version_matrix!(too_many_bytes_are_relayed_untouched; p764 = 764, p777 = 777);

async fn an_undecodable_tree_replaces_the_previous_one(version: ProtocolVersion) {
    let id = wire::packet_id::<CCommands>(version).unwrap();
    let out_of_tree_root = Bytes::from_static(&[1, 0, 0, 5]);
    relayed_as_is_and_never_refreshed(version, PacketFrame::new(id, out_of_tree_root)).await;
}

version_matrix!(an_undecodable_tree_replaces_the_previous_one; p764 = 764, p777 = 777);

struct Spread {
    lobby: FakeBackend,
    a: FakeBackend,
    b: FakeBackend,
    c: FakeBackend,
    proxy: TestProxy,
}

impl Spread {
    async fn start() -> Self {
        let lobby = FakeBackend::builder().hold_config().spawn().await.unwrap();
        let a = FakeBackend::builder().spawn().await.unwrap();
        let b = FakeBackend::builder().spawn().await.unwrap();
        let c = FakeBackend::builder().spawn().await.unwrap();
        let proxy = TestProxy::builder()
            .server(ServerSpec::offline("lobby").backend(lobby.addr()))
            .server(ServerSpec::offline("a").backend(a.addr()))
            .server(ServerSpec::offline("b").backend(b.addr()))
            .server(ServerSpec::offline("c").backend(c.addr()))
            .start()
            .await
            .unwrap();
        Self {
            lobby,
            a,
            b,
            c,
            proxy,
        }
    }
}

async fn only_the_last_switch_during_configuration_runs(version: ProtocolVersion) {
    let world = Spread::start().await;
    let client = world.proxy.client_for("lobby", version).unwrap();
    let login = tokio::spawn(async move { client.login("Steve").await });
    let mut conn = world.lobby.next_connection(T).await.unwrap();
    let player = world.proxy.wait_for_player("Steve", T).await.unwrap();

    for target in ["a", "b", "c"] {
        player.switch_server(ServerId::new(target)).await.unwrap();
    }
    conn.finish_config(T).await.unwrap();
    let mut session = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();

    session.expect_join(T).await.unwrap();
    world.c.next_connection(T).await.unwrap();
    assert_eq!(world.a.accepted_connections(), 0);
    assert_eq!(world.b.accepted_connections(), 0);

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(only_the_last_switch_during_configuration_runs; p764 = 764, p777 = 777);

async fn a_replaced_connect_is_cancelled(version: ProtocolVersion) {
    let world = Spread::start().await;
    let client = world.proxy.client_for("lobby", version).unwrap();
    let login = tokio::spawn(async move { client.login("Steve").await });
    let mut conn = world.lobby.next_connection(T).await.unwrap();
    let player = world.proxy.wait_for_player("Steve", T).await.unwrap();

    let mut replaced = player.connect(ServerId::new("a"));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut replaced)
            .await
            .is_err(),
        "the switch waits for the end of the configuration phase"
    );
    player.switch_server(ServerId::new("b")).await.unwrap();
    assert_eq!(
        tokio::time::timeout(T, replaced).await.unwrap().unwrap(),
        ConnectionResult::Cancelled
    );

    conn.finish_config(T).await.unwrap();
    let mut session = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();
    session.expect_join(T).await.unwrap();
    assert_eq!(world.a.accepted_connections(), 0);

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(a_replaced_connect_is_cancelled; p764 = 764, p777 = 777);
