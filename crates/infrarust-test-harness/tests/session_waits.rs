#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::command::{
    CommandContext, CommandHandler, CommandSpec, SuggestContext, Suggestion,
};
use infrarust_api::error::PlayerError;
use infrarust_api::event::BoxFuture;
use infrarust_api::player::{ConnectionResult, Player};
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::packets::play::chat::{CChatMessageLegacy, CSystemChatMessage};
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::tab_complete::{CTabCompleteResponse, STabCompleteRequest};
use tokio::sync::{Notify, mpsc, oneshot};

use infrarust_test_harness::text::component_text;
use infrarust_test_harness::{
    ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, PacketFrame, ProtocolVersion, Recorder,
    ScriptedPlugin, ServerSpec, TestProxy, version_matrix, wire,
};

const T: Duration = DEFAULT_TIMEOUT;

type Log = Arc<Mutex<Vec<String>>>;

fn lines(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
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

async fn joined_and_told(session: &mut ClientSession, expected: &str) {
    let version = session.version();
    let (mut joined, mut told) = (false, false);
    while !(joined && told) {
        let frame = session
            .recv_frame(T)
            .await
            .unwrap_or_else(|e| panic!("joined {joined}, told {expected:?} {told}: {e}"));
        joined |= wire::is::<CJoinGame>(&frame, version);
        told |= system_text(&frame, version).as_deref() == Some(expected);
    }
}

async fn two_servers(
    a: &FakeBackend,
    b: &FakeBackend,
    plugin: ScriptedPlugin,
    recorder: &Recorder,
) -> TestProxy {
    TestProxy::builder()
        .server(ServerSpec::offline("a").backend(a.addr()).network("main"))
        .server(ServerSpec::offline("b").backend(b.addr()).network("main"))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap()
}

async fn join(proxy: &TestProxy, version: ProtocolVersion) -> (ClientSession, Arc<dyn Player>) {
    let session = proxy
        .client_for("a", version)
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    (session, player)
}

fn outcome(result: &Result<ConnectionResult, PlayerError>) -> String {
    match result {
        Ok(result) => result.as_str().to_owned(),
        Err(error) => format!("error {error}"),
    }
}

struct Hop;

impl CommandHandler for Hop {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player) = ctx.source.player().cloned() else {
                return;
            };
            let result = player.connect(ServerId::new("b")).await;
            ctx.source
                .send_message(Component::text(format!("hop: {}", outcome(&result))));
        })
    }
}

fn hopper() -> ScriptedPlugin {
    ScriptedPlugin::new("hopper").on_enable(|ctx| {
        ctx.command_manager()
            .register(CommandSpec::new("hop"), Box::new(Hop))
            .unwrap();
    })
}

async fn a_command_can_wait_for_its_own_players_switch(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = two_servers(&backend_a, &backend_b, hopper(), &recorder).await;
    let (mut session, player) = join(&proxy, version).await;

    session.command("hop").await.unwrap();

    joined_and_told(&mut session, "hop: success").await;
    let _conn_b = backend_b.next_connection(T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("b")));

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_command_can_wait_for_its_own_players_switch; p47 = 47, p764 = 764, p774 = 774);

struct Step {
    name: &'static str,
    log: Log,
    started: mpsc::UnboundedSender<&'static str>,
    gate: Option<Arc<Notify>>,
}

impl CommandHandler for Step {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.log
                .lock()
                .unwrap()
                .push(format!("{} start", self.name));
            let _ = self.started.send(self.name);
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            self.log.lock().unwrap().push(format!("{} end", self.name));
            ctx.source
                .send_message(Component::text(format!("{} done", self.name)));
        })
    }
}

async fn a_players_commands_run_one_at_a_time_in_order(version: ProtocolVersion) {
    let log = Log::default();
    let gate = Arc::new(Notify::new());
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let steps = {
        let log = Arc::clone(&log);
        let gate = Arc::clone(&gate);
        ScriptedPlugin::new("steps").on_enable(move |ctx| {
            for (name, gate) in [("first", Some(Arc::clone(&gate))), ("second", None)] {
                let step = Step {
                    name,
                    log: Arc::clone(&log),
                    started: started_tx.clone(),
                    gate,
                };
                ctx.command_manager()
                    .register(CommandSpec::new(name), Box::new(step))
                    .unwrap();
            }
        })
    };
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = two_servers(&backend_a, &backend_b, steps, &recorder).await;
    let (mut session, _player) = join(&proxy, version).await;
    let mut conn = backend_a.next_connection(T).await.unwrap();

    session.command("first").await.unwrap();
    session.command("second").await.unwrap();
    session.chat("after both").await.unwrap();

    let first = tokio::time::timeout(T, started.recv())
        .await
        .expect("the first command starts")
        .unwrap();
    assert_eq!(first, "first");
    conn.chat_until("after both", T)
        .await
        .expect("the session keeps forwarding while the first command runs");
    assert!(
        started.try_recv().is_err(),
        "the second command must wait for the first: {:?}",
        lines(&log)
    );

    gate.notify_one();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "first done");
    assert_eq!(session.expect_system_text(T).await.unwrap(), "second done");
    assert_eq!(
        lines(&log),
        ["first start", "first end", "second start", "second end"]
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_players_commands_run_one_at_a_time_in_order; p47 = 47, p774 = 774);

struct DropSignal(Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

struct Stuck {
    started: mpsc::UnboundedSender<()>,
    dropped: Mutex<Option<oneshot::Sender<()>>>,
}

impl CommandHandler for Stuck {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        let signal = DropSignal(self.dropped.lock().unwrap().take());
        Box::pin(async move {
            let _signal = signal;
            let _source = ctx.source;
            let _ = self.started.send(());
            std::future::pending::<()>().await;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_still_running_when_the_player_leaves_is_cancelled() {
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let (dropped_tx, dropped) = oneshot::channel();
    let stuck = Arc::new(Stuck {
        started: started_tx,
        dropped: Mutex::new(Some(dropped_tx)),
    });
    let plugin = {
        let stuck = Arc::clone(&stuck);
        ScriptedPlugin::new("stuck").on_enable(move |ctx| {
            ctx.command_manager()
                .register(
                    CommandSpec::new("stuck"),
                    Box::new(Forward(Arc::clone(&stuck))),
                )
                .unwrap();
        })
    };
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = two_servers(&backend_a, &backend_b, plugin, &recorder).await;
    let (session, player) = join(&proxy, ProtocolVersion(774)).await;
    let weak = Arc::downgrade(&player);
    drop(player);

    session.command("stuck").await.unwrap();
    tokio::time::timeout(T, started.recv())
        .await
        .expect("the command starts")
        .unwrap();
    session.quit().await;

    proxy
        .wait_for_connection_count(0, T)
        .await
        .expect("a running command must not keep the player online");
    recorder
        .wait_for_kind(EventKind::Disconnect, T)
        .await
        .unwrap();
    tokio::time::timeout(T, dropped)
        .await
        .expect("the command still running is cancelled with the session")
        .unwrap();

    proxy.shutdown().await.unwrap();
    assert!(
        weak.upgrade().is_none(),
        "nothing keeps the departed player alive"
    );
}

struct Forward(Arc<Stuck>);

impl CommandHandler for Forward {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        self.0.execute(ctx)
    }
}

struct SlowCompleter {
    started: mpsc::UnboundedSender<()>,
    gate: Arc<Notify>,
}

impl CommandHandler for SlowCompleter {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        Box::pin(async move {
            let _ = self.started.send(());
            self.gate.notified().await;
            vec![Suggestion::new(format!("{}rld", ctx.partial()))]
        })
    }
}

struct Shared<H>(Arc<H>);

impl<H: CommandHandler> CommandHandler for Shared<H> {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        self.0.execute(ctx)
    }

    fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        self.0.suggest(ctx)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_completer_does_not_hold_the_players_session() {
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let gate = Arc::new(Notify::new());
    let completer = Arc::new(SlowCompleter {
        started: started_tx,
        gate: Arc::clone(&gate),
    });
    let plugin = ScriptedPlugin::new("teleport").on_enable(move |ctx| {
        ctx.command_manager()
            .register(
                CommandSpec::new("tp"),
                Box::new(Shared(Arc::clone(&completer))),
            )
            .unwrap();
    });
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = two_servers(&backend_a, &backend_b, plugin, &recorder).await;
    let (mut session, _player) = join(&proxy, ProtocolVersion(774)).await;
    let mut conn = backend_a.next_connection(T).await.unwrap();

    session
        .send_packet(&STabCompleteRequest {
            transaction_id: 4,
            text: "/tp wo".into(),
        })
        .await
        .unwrap();
    tokio::time::timeout(T, started.recv())
        .await
        .expect("the completer starts")
        .unwrap();
    session.chat("while completing").await.unwrap();
    conn.chat_until("while completing", T)
        .await
        .expect("the session keeps forwarding while the completer runs");

    gate.notify_one();
    let response = session.expect::<CTabCompleteResponse>(T).await.unwrap();
    assert_eq!(response.transaction_id, 4);
    assert_eq!((response.start, response.length), (4, 2));
    assert_eq!(response.matches.len(), 1);
    assert_eq!(response.matches[0].text, "world");

    proxy.shutdown().await.unwrap();
}
