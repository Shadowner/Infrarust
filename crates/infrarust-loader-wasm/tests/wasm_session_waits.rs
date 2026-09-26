#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(all(feature = "wasm", wasm_fixtures_available))]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use infrarust_api::types::ServerId;
use infrarust_protocol::packets::play::chat::{CChatMessageLegacy, CSystemChatMessage};
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_test_harness::text::component_text;
use infrarust_test_harness::{
    ClientSession, FakeBackend, PacketFrame, ProtocolVersion, ServerSpec, TestProxy, wire,
};
use tokio::time::Instant;
use toml::Value;
use toml::value::Table;

use support::{add_fixture, fresh_loader, read_log, write_script};

const T: Duration = infrarust_test_harness::DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion::V1_21;

fn grant_chat(plugins_dir: PathBuf) -> impl FnOnce(&mut Table) + Send + 'static {
    move |table| {
        table.insert(
            "plugins_dir".into(),
            Value::String(plugins_dir.to_string_lossy().into_owned()),
        );
        let grants = Table::from_iter([(
            "permissions".to_owned(),
            Value::Array(vec![Value::String("chat-intercept".into())]),
        )]);
        table.insert(
            "plugins".into(),
            Value::Table(Table::from_iter([(
                "scripted".to_owned(),
                Value::Table(grants),
            )])),
        );
    }
}

struct World {
    proxy: TestProxy,
    backend_a: FakeBackend,
    backend_b: FakeBackend,
    _dir: tempfile::TempDir,
    data: PathBuf,
}

impl World {
    async fn start(script: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let plugins_dir = dir.path().to_path_buf();
        add_fixture(&plugins_dir, "scripted", "scripted");
        write_script(&plugins_dir, "scripted", script);
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
            .loader(Box::new(fresh_loader()))
            .patch_config(grant_chat(plugins_dir.clone()))
            .start()
            .await
            .unwrap();
        assert!(
            proxy.plugin_context("scripted").await.is_some(),
            "the WASM plugin is enabled"
        );
        Self {
            proxy,
            backend_a,
            backend_b,
            data: plugins_dir.join("scripted"),
            _dir: dir,
        }
    }

    async fn join(&self) -> ClientSession {
        let session = self
            .proxy
            .client_for("a", VERSION)
            .unwrap()
            .login("Steve")
            .await
            .unwrap()
            .joined()
            .unwrap();
        self.proxy.wait_for_player("Steve", T).await.unwrap();
        session
    }
}

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

fn logged(data: &Path, line: &str) -> bool {
    read_log(data).iter().any(|seen| seen == line)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_command_can_wait_for_its_own_players_switch() {
    let world = World::start("cmd hop connect b").await;
    let mut session = world.join().await;
    let _conn_a = world.backend_a.next_connection(T).await.unwrap();

    session.command("hop").await.unwrap();

    let (mut joined, mut told) = (false, false);
    while !(joined && told) {
        let frame = session
            .recv_frame(T)
            .await
            .unwrap_or_else(|e| panic!("joined {joined}, told {told}: {e}"));
        joined |= wire::is::<CJoinGame>(&frame, VERSION);
        told |= system_text(&frame, VERSION).as_deref() == Some("hop success");
    }
    let _conn = world.backend_b.next_connection(T).await.unwrap();
    let player = world.proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("b")));
    assert!(
        logged(&world.data, "cmd hop connect b success"),
        "{:?}",
        read_log(&world.data)
    );

    world.proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_listener_waiting_for_its_own_players_switch_fails_at_once() {
    let world = World::start("on chat-message normal connect b").await;
    let session = world.join().await;
    let mut conn = world.backend_a.next_connection(T).await.unwrap();

    let sent = Instant::now();
    session.chat("hello").await.unwrap();
    conn.chat_until("hello", T)
        .await
        .expect("the chat goes on long before host_call_timeout");
    let waited = sent.elapsed();

    assert!(
        waited < Duration::from_secs(1),
        "the chat waited {waited:?}"
    );
    assert!(
        logged(&world.data, "chat-message connect b invalid-state"),
        "{:?}",
        read_log(&world.data)
    );
    let player = world.proxy.wait_for_player("Steve", T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("a")));

    world.proxy.shutdown().await.unwrap();
}
