#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::types::Component;
use infrarust_protocol::packets::play::chat::SChatMessage;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use infrarust_test_harness::chat::{self, ChatFrame, ChatPacket};
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, ProtocolVersion, Recorder,
    ScriptedPlugin, ServerSpec, TestProxy, version_matrix, wire,
};

const T: Duration = DEFAULT_TIMEOUT;
const MARKER: &str = "marker";
const OFFSET: i32 = 3;
const HOLD: &str = "chat-hold";

fn moderator() -> ScriptedPlugin {
    ScriptedPlugin::new("moderator").on::<ChatMessageEvent>(EventPriority::NORMAL, |event| {
        if let Some(rest) = event.message.strip_prefix("deny ") {
            event.deny(Component::text(format!("no {rest}")));
        } else if event.message.starts_with("silent ") {
            event.deny_silently();
        } else if let Some(rest) = event.message.strip_prefix("modify ") {
            event.modify(format!("modified {rest}"));
        }
    })
}

struct World {
    proxy: TestProxy,
    backend: FakeBackend,
    recorder: Recorder,
}

impl World {
    async fn start(plugin: ScriptedPlugin) -> Self {
        let recorder = Recorder::new();
        let backend = FakeBackend::builder().spawn().await.unwrap();
        let proxy = TestProxy::builder()
            .server(ServerSpec::offline("lobby").backend(backend.addr()))
            .plugin(plugin)
            .plugin(recorder.plugin())
            .start()
            .await
            .unwrap();
        Self {
            proxy,
            backend,
            recorder,
        }
    }

    async fn join(&self, version: ProtocolVersion) -> (ClientSession, BackendConn) {
        let session = self
            .proxy
            .client(version)
            .login("Steve")
            .await
            .unwrap()
            .joined()
            .unwrap();
        let conn = self.backend.next_connection(T).await.unwrap();
        self.proxy.wait_for_player("Steve", T).await.unwrap();
        (session, conn)
    }

    fn chats(&self) -> Vec<Value> {
        self.recorder
            .of(EventKind::ChatMessage)
            .into_iter()
            .map(|event| event.detail)
            .collect()
    }
}

fn packets(seen: &[ChatFrame]) -> Vec<ChatPacket> {
    seen.iter().map(|chat| chat.packet.clone()).collect()
}

fn raw(seen: &[ChatFrame]) -> Vec<(i32, Vec<u8>)> {
    seen.iter()
        .map(|chat| (chat.frame.id, chat.frame.payload.to_vec()))
        .collect()
}

fn acknowledgements(offset: i32, version: ProtocolVersion) -> Vec<ChatPacket> {
    chat::acknowledgement(offset, version)
        .map(ChatPacket::Acknowledgement)
        .into_iter()
        .collect()
}

fn signs(version: ProtocolVersion) -> bool {
    version.no_less_than(ProtocolVersion::V1_19)
}

async fn allowed_chat_is_forwarded_byte_for_byte(version: ProtocolVersion) {
    let world = World::start(moderator()).await;
    let (session, mut conn) = world.join(version).await;

    let unsigned = wire::encode(&chat::unsigned_chat("hello", OFFSET, version), version).unwrap();
    session.send_frame(&unsigned).await.unwrap();
    let signed = session.chat_signed("hello again", OFFSET).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(
        raw(&seen),
        [
            (unsigned.id, unsigned.payload.to_vec()),
            (signed.id, signed.payload.to_vec())
        ]
    );
    assert_eq!(
        world.chats()[..2],
        [
            json!({ "message": "hello", "signed": false, "server": "lobby", "result": "allow" }),
            json!({
                "message": "hello again",
                "signed": signs(version),
                "server": "lobby",
                "result": "allow",
            }),
        ]
    );

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, allowed_chat_is_forwarded_byte_for_byte);

async fn denied_chat_is_dropped_acknowledged_and_explained(version: ProtocolVersion) {
    let world = World::start(moderator()).await;
    let (mut session, mut conn) = world.join(version).await;

    session.chat_signed("deny this", OFFSET).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(packets(&seen), acknowledgements(OFFSET, version));
    assert_eq!(session.expect_system_text(T).await.unwrap(), "no this");
    assert_eq!(world.chats()[0]["result"], json!({ "deny": "no this" }));

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, denied_chat_is_dropped_acknowledged_and_explained);

async fn silently_denied_chat_tells_nobody(version: ProtocolVersion) {
    let world = World::start(moderator()).await;
    let (mut session, mut conn) = world.join(version).await;

    session.chat_signed("silent please", OFFSET).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(packets(&seen), acknowledgements(OFFSET, version));
    conn.send_system_message_json(r#"{"text":"after"}"#)
        .await
        .unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "after",
        "the silent denial showed the player a message"
    );
    assert_eq!(world.chats()[0]["result"], json!({ "deny": null }));

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, silently_denied_chat_tells_nobody);

async fn modified_chat_reaches_the_backend_rewritten(version: ProtocolVersion) {
    let world = World::start(moderator()).await;
    let (session, mut conn) = world.join(version).await;

    let original = chat::unsigned_chat("modify this", OFFSET, version);
    session.send_packet(&original).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    let expected = SChatMessage {
        message: "modified this".to_string(),
        ..original
    };
    assert_eq!(packets(&seen), [ChatPacket::Message(expected)]);

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, modified_chat_reaches_the_backend_rewritten);

async fn modified_signed_chat_is_sent_unsigned(version: ProtocolVersion) {
    let world = World::start(moderator()).await;
    let (session, mut conn) = world.join(version).await;

    let original = chat::signed_chat("modify this", OFFSET, version);
    session.send_packet(&original).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    let expected = SChatMessage {
        message: "modified this".to_string(),
        signature: None,
        signed_preview: false,
        ..original
    };
    assert_eq!(packets(&seen), [ChatPacket::Message(expected)]);
    assert_eq!(world.chats()[0]["signed"], json!(true));

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(modified_signed_chat_is_sent_unsigned; p760 = 760, p762 = 762, p770 = 770, p774 = 774);

struct ChatLimbo {
    chats: mpsc::UnboundedSender<String>,
}

impl LimboHandler for ChatLimbo {
    fn name(&self) -> &str {
        HOLD
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

async fn next_chat(chats: &mut mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(T, chats.recv())
        .await
        .expect("the limbo handler saw a chat message")
        .unwrap()
}

async fn limbo_chat_goes_through_the_event_first(version: ProtocolVersion) {
    let (tx, mut chats) = mpsc::unbounded_channel();
    let gate = ScriptedPlugin::new("gate").on_enable(move |ctx| {
        ctx.register_limbo_handler(Box::new(ChatLimbo { chats: tx.clone() }))
            .expect("the limbo handler registers");
    });
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("hub")
                .unreachable()
                .limbo_handlers([HOLD]),
        )
        .plugin(moderator())
        .plugin(gate)
        .plugin(recorder.plugin())
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
    proxy.wait_for_player("Steve", T).await.unwrap();

    session.chat("deny secret").await.unwrap();
    session.chat("modify me").await.unwrap();
    session.chat("hello").await.unwrap();

    assert_eq!(next_chat(&mut chats).await, "modified me");
    assert_eq!(next_chat(&mut chats).await, "hello");
    assert_eq!(session.expect_system_text(T).await.unwrap(), "no secret");
    let seen: Vec<Value> = recorder
        .of(EventKind::ChatMessage)
        .into_iter()
        .map(|event| json!([event.detail["message"], event.detail["server"]]))
        .collect();
    assert_eq!(
        seen,
        [
            json!(["deny secret", null]),
            json!(["modify me", null]),
            json!(["hello", null]),
        ]
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(limbo_chat_goes_through_the_event_first; p340 = 340, p764 = 764, p774 = 774);
