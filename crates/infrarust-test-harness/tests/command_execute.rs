#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::command::CommandExecuteEvent;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::types::Component;
use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use infrarust_test_harness::chat::{self, ChatFrame, ChatPacket};
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, PacketFrame,
    ProtocolVersion, Recorder, ScriptedPlugin, ServerSpec, TestProxy, version_matrix,
};

const T: Duration = DEFAULT_TIMEOUT;
const MARKER: &str = "marker";
const OFFSET: i32 = 5;
const HOLD: &str = "command-hold";

type Ran = Arc<Mutex<Vec<String>>>;

struct Probe {
    ran: Ran,
}

impl CommandHandler for Probe {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        self.ran
            .lock()
            .unwrap()
            .push(format!("{} [{}]", ctx.label, ctx.args.join(",")));
        ctx.source.send_message(Component::text("ran open"));
        Box::pin(async {})
    }
}

fn owner(ran: &Ran) -> ScriptedPlugin {
    let ran = Arc::clone(ran);
    ScriptedPlugin::new("owner").on_enable(move |ctx| {
        ctx.command_manager()
            .register(
                CommandSpec::new("open"),
                Box::new(Probe {
                    ran: Arc::clone(&ran),
                }),
            )
            .unwrap();
    })
}

fn gatekeeper() -> ScriptedPlugin {
    ScriptedPlugin::new("gatekeeper").on::<CommandExecuteEvent>(EventPriority::NORMAL, |event| {
        match event.label() {
            "blocked" => event.deny(Component::text("no blocked")),
            "quiet" => event.deny_silently(),
            "rewrite" => event.modify("vanilla rewritten"),
            "reopen" => event.modify("open rewritten"),
            "open" if event.command.ends_with("backend") => event.forward_to_backend(),
            _ => {}
        }
    })
}

struct World {
    proxy: TestProxy,
    backend: FakeBackend,
    recorder: Recorder,
    ran: Ran,
}

impl World {
    async fn start() -> Self {
        let ran = Ran::default();
        let recorder = Recorder::new();
        let backend = FakeBackend::builder().spawn().await.unwrap();
        let proxy = TestProxy::builder()
            .server(ServerSpec::offline("lobby").backend(backend.addr()))
            .plugin(owner(&ran))
            .plugin(gatekeeper())
            .plugin(recorder.plugin())
            .start()
            .await
            .unwrap();
        Self {
            proxy,
            backend,
            recorder,
            ran,
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

    fn ran(&self) -> Vec<String> {
        std::mem::take(&mut *self.ran.lock().unwrap())
    }

    fn commands(&self) -> Vec<Value> {
        self.recorder
            .of(EventKind::CommandExecute)
            .into_iter()
            .map(|event| event.detail)
            .collect()
    }
}

fn packets(seen: &[ChatFrame]) -> Vec<ChatPacket> {
    seen.iter().map(|chat| chat.packet.clone()).collect()
}

fn raw(frames: &[&PacketFrame]) -> Vec<(i32, Vec<u8>)> {
    frames
        .iter()
        .map(|frame| (frame.id, frame.payload.to_vec()))
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

async fn command_event_describes_the_command(version: ProtocolVersion) {
    let world = World::start().await;
    let (session, mut conn) = world.join(version).await;

    session.command("vanilla arg").await.unwrap();
    let signed = session.command_signed("vanilla signed", 0).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0].command(), Some("vanilla arg"));
    assert_eq!(
        raw(&[&seen[1].frame]),
        raw(&[&signed]),
        "an allowed command is forwarded byte for byte"
    );
    assert_eq!(
        world.commands(),
        [
            json!({ "command": "vanilla arg", "signed": false, "server": "lobby", "result": "allow" }),
            json!({
                "command": "vanilla signed",
                "signed": signs(version),
                "server": "lobby",
                "result": "allow",
            }),
        ]
    );
    assert!(world.ran().is_empty());

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, command_event_describes_the_command);

async fn denied_command_is_dropped_acknowledged_and_explained(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;

    session.command_signed("blocked now", OFFSET).await.unwrap();
    session.command("quiet please").await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(packets(&seen), acknowledgements(OFFSET, version));
    conn.send_system_message_json(r#"{"text":"after"}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "no blocked");
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "after",
        "the silent denial showed the player a message"
    );
    let results: Vec<Value> = world
        .commands()
        .into_iter()
        .map(|detail| detail["result"].clone())
        .collect();
    assert_eq!(
        results,
        [json!({ "deny": "no blocked" }), json!({ "deny": null })]
    );

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, denied_command_is_dropped_acknowledged_and_explained);

fn rewritten(sent: &PacketFrame, version: ProtocolVersion) -> Vec<ChatPacket> {
    let original = ChatFrame::classify(sent, version).unwrap().unwrap().packet;
    match original {
        ChatPacket::Message(_) => vec![ChatPacket::Message(SChatMessage {
            message: "/vanilla rewritten".to_string(),
            ..SChatMessage::default()
        })],
        ChatPacket::Command(original) => vec![ChatPacket::Command(SChatCommand {
            command: "vanilla rewritten".to_string(),
            argument_signatures: Vec::new(),
            signed_preview: false,
            ..original
        })],
        ChatPacket::SignedCommand(_) => {
            let mut expected = acknowledgements(OFFSET, version);
            expected.push(ChatPacket::Command(SChatCommand {
                command: "vanilla rewritten".to_string(),
                ..SChatCommand::default()
            }));
            expected
        }
        ChatPacket::Acknowledgement(other) => panic!("the client sent {other:?}"),
    }
}

async fn modified_command_reaches_the_backend_unsigned(version: ProtocolVersion) {
    let world = World::start().await;
    let (session, mut conn) = world.join(version).await;

    let sent = session.command_signed("rewrite now", OFFSET).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(packets(&seen), rewritten(&sent, version));
    assert_eq!(
        world.commands()[0]["result"],
        json!({ "modify": "vanilla rewritten" })
    );

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, modified_command_reaches_the_backend_unsigned);

async fn a_command_modified_into_a_proxy_command_runs_on_the_proxy(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;

    session.command_signed("reopen now", OFFSET).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(packets(&seen), acknowledgements(OFFSET, version));
    assert_eq!(session.expect_system_text(T).await.unwrap(), "ran open");
    assert_eq!(world.ran(), ["open [rewritten]"]);

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(a_command_modified_into_a_proxy_command_runs_on_the_proxy; p340 = 340, p760 = 760, p762 = 762, p774 = 774);

async fn forward_to_backend_skips_the_proxy_command(version: ProtocolVersion) {
    let world = World::start().await;
    let (session, mut conn) = world.join(version).await;

    let sent = session
        .command_signed("open backend", OFFSET)
        .await
        .unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    let frames: Vec<&PacketFrame> = seen.iter().map(|chat| &chat.frame).collect();
    assert_eq!(raw(&frames), raw(&[&sent]));
    assert!(world.ran().is_empty(), "the proxy ran a forwarded command");
    assert_eq!(world.commands()[0]["result"], json!("forward_to_backend"));

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(CHAT, forward_to_backend_skips_the_proxy_command);

async fn signed_proxy_command_runs_on_the_proxy(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;

    session.command_signed("open now", 0).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(
        packets(&seen),
        Vec::<ChatPacket>::new(),
        "the proxy command reached the backend"
    );
    assert_eq!(session.expect_system_text(T).await.unwrap(), "ran open");
    assert_eq!(world.ran(), ["open [now]"]);
    assert_eq!(
        world.commands(),
        [json!({ "command": "open now", "signed": true, "server": "lobby", "result": "allow" })]
    );

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(signed_proxy_command_runs_on_the_proxy; p766 = 766, p770 = 770, p774 = 774);

async fn proxy_command_acknowledges_the_offset(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;

    session.command_signed("open now", OFFSET).await.unwrap();
    session.chat(MARKER).await.unwrap();

    let seen = conn.chat_until(MARKER, T).await.unwrap();
    assert_eq!(packets(&seen), acknowledgements(OFFSET, version));
    assert_eq!(session.expect_system_text(T).await.unwrap(), "ran open");
    assert_eq!(world.ran(), ["open [now]"]);

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(proxy_command_acknowledges_the_offset; p760 = 760, p762 = 762, p770 = 770, p774 = 774);

struct CommandLimbo {
    commands: mpsc::UnboundedSender<String>,
}

impl LimboHandler for CommandLimbo {
    fn name(&self) -> &str {
        HOLD
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async { HandlerResult::Hold })
    }

    fn on_command<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        let _ = self.commands.send(format!("{command} {}", args.join(" ")));
        Box::pin(async {})
    }
}

async fn limbo_commands_fire_no_command_event(version: ProtocolVersion) {
    let (tx, mut commands) = mpsc::unbounded_channel();
    let gate = ScriptedPlugin::new("gate").on_enable(move |ctx| {
        ctx.register_limbo_handler(Box::new(CommandLimbo {
            commands: tx.clone(),
        }));
    });
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("hub")
                .unreachable()
                .limbo_handlers([HOLD]),
        )
        .plugin(gatekeeper())
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();

    session.command("login hunter2").await.unwrap();
    session.command("blocked anyway").await.unwrap();

    for expected in ["login hunter2", "blocked anyway"] {
        let reached = tokio::time::timeout(T, commands.recv())
            .await
            .expect("the limbo handler saw the command")
            .unwrap();
        assert_eq!(reached, expected);
    }
    assert_eq!(recorder.count(EventKind::CommandExecute), 0);

    proxy.shutdown().await.unwrap();
}

version_matrix!(limbo_commands_fire_no_command_event; p340 = 340, p764 = 764, p774 = 774);
