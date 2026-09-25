#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::command::{
    CommandContext, CommandHandler, CommandManager, CommandSpec, SuggestContext, Suggestion,
};
use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::types::Component;
use infrarust_core::console::ConsoleServices;
use infrarust_core::console::commands::register_all;
use infrarust_core::console::dispatcher::CommandDispatcher;
use infrarust_core::console::output::CommandOutput;
use infrarust_core::services::config_service::ConfigServiceImpl;
use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
use infrarust_protocol::packets::play::commands::{CCommands, CommandNode};
use infrarust_protocol::packets::play::tab_complete::{CTabCompleteResponse, STabCompleteRequest};
use tokio::sync::mpsc;

use infrarust_test_harness::text::component_text;
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin,
    ServerSpec, TestProxy, version_matrix, wire,
};

const T: Duration = DEFAULT_TIMEOUT;
const PLUGIN: &str = "tester";
const HOLD: &str = "hold";

type Seen = Arc<Mutex<Vec<String>>>;
type Handle = Arc<Mutex<Option<Arc<dyn CommandManager>>>>;

struct Probe {
    tag: &'static str,
    seen: Seen,
}

impl CommandHandler for Probe {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        self.seen.lock().unwrap().push(format!(
            "{} {} {} [{}]",
            self.tag,
            ctx.source.name(),
            ctx.label,
            ctx.args.join(",")
        ));
        ctx.source
            .send_message(Component::text(format!("ran {}", self.tag)));
        Box::pin(async {})
    }

    fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        let completion = format!("{}rld", ctx.partial());
        let tooltip = format!("for {}", ctx.source.name());
        Box::pin(
            async move { vec![Suggestion::new(completion).with_tooltip(Component::text(tooltip))] },
        )
    }
}

fn probe(tag: &'static str, seen: &Seen) -> Box<dyn CommandHandler> {
    Box::new(Probe {
        tag,
        seen: Arc::clone(seen),
    })
}

fn tester(seen: &Seen, handle: &Handle) -> ScriptedPlugin {
    let seen = Arc::clone(seen);
    let handle = Arc::clone(handle);
    ScriptedPlugin::new(PLUGIN).on_enable(move |ctx| {
        let commands = ctx.command_manager();
        commands
            .register(CommandSpec::new("open").alias("opn"), probe("open", &seen))
            .unwrap();
        commands
            .register(
                CommandSpec::new("secret").permission("tester.secret"),
                probe("secret", &seen),
            )
            .unwrap();
        commands
            .register(
                CommandSpec::new("quiet").hidden(true),
                probe("quiet", &seen),
            )
            .unwrap();
        *handle.lock().unwrap() = Some(ctx.command_manager_handle());
    })
}

struct World {
    proxy: TestProxy,
    backend: FakeBackend,
    seen: Seen,
    handle: Handle,
}

impl World {
    async fn start() -> Self {
        let seen = Seen::default();
        let handle = Handle::default();
        let backend = FakeBackend::builder().spawn().await.unwrap();
        let proxy = TestProxy::builder()
            .server(ServerSpec::offline("lobby").backend(backend.addr()))
            .plugin(tester(&seen, &handle))
            .start()
            .await
            .unwrap();
        Self {
            proxy,
            backend,
            seen,
            handle,
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

    fn commands(&self) -> Arc<dyn CommandManager> {
        self.handle.lock().unwrap().clone().expect("tester enabled")
    }

    fn seen(&self) -> Vec<String> {
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

fn tree(roots: &[&str]) -> CCommands {
    let mut nodes = vec![CommandNode {
        flags: 0,
        children: vec![],
        redirect_node: None,
        name: None,
        parser: None,
        suggestions_type: None,
    }];
    for (i, name) in roots.iter().enumerate() {
        nodes.push(CommandNode::literal_executable(name));
        nodes[0].children.push(i32::try_from(i + 1).unwrap());
    }
    CCommands {
        nodes,
        root_index: 0,
    }
}

fn roots(tree: &CCommands) -> Vec<String> {
    let mut names: Vec<String> = tree.nodes[tree.root_index as usize]
        .children
        .iter()
        .filter_map(|&i| tree.nodes[i as usize].name.clone())
        .collect();
    names.sort_unstable();
    names
}

fn has(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n == name)
}

fn forwarded_commands(conn: &BackendConn, version: ProtocolVersion) -> Vec<String> {
    conn.received()
        .iter()
        .filter_map(|frame| {
            if wire::is::<SChatCommand>(frame, version) {
                return Some(
                    wire::decode::<SChatCommand>(frame, version)
                        .unwrap()
                        .command,
                );
            }
            if wire::is::<SChatMessage>(frame, version) {
                let message = wire::decode::<SChatMessage>(frame, version)
                    .unwrap()
                    .message;
                return message.strip_prefix('/').map(str::to_string);
            }
            None
        })
        .collect()
}

async fn wait_forwarded(conn: &mut BackendConn, version: ProtocolVersion, expected: &str) {
    let deadline = tokio::time::Instant::now() + T;
    while !forwarded_commands(conn, version)
        .iter()
        .any(|c| c == expected)
    {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        conn.recv_frame(left)
            .await
            .unwrap_or_else(|e| panic!("/{expected} never reached the backend: {e}"));
    }
}

async fn late_command_reaches_the_tree(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;
    conn.send_packet(&tree(&[])).await.unwrap();

    let first = roots(&session.expect::<CCommands>(T).await.unwrap());
    for name in ["open", "opn", "tester:open"] {
        assert!(has(&first, name), "missing {name}: {first:?}");
    }
    for name in ["secret", "tester:secret", "quiet", "late"] {
        assert!(!has(&first, name), "{name} must not be shown: {first:?}");
    }

    world
        .commands()
        .register(
            CommandSpec::new("late").alias("lt"),
            probe("late", &world.seen),
        )
        .unwrap();
    let refreshed = roots(&session.expect::<CCommands>(T).await.unwrap());
    for name in ["late", "lt", "tester:late", "open"] {
        assert!(has(&refreshed, name), "missing {name}: {refreshed:?}");
    }

    world.commands().unregister("late").unwrap();
    let pruned = roots(&session.expect::<CCommands>(T).await.unwrap());
    assert!(!has(&pruned, "late") && !has(&pruned, "lt"), "{pruned:?}");
    assert!(has(&pruned, "open"), "{pruned:?}");

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(late_command_reaches_the_tree; p764 = 764, p774 = 774);

async fn backend_root_with_a_proxy_name_appears_once(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;
    conn.send_packet(&tree(&["open", "gamemode", "secret"]))
        .await
        .unwrap();

    let seen = roots(&session.expect::<CCommands>(T).await.unwrap());
    assert_eq!(seen.iter().filter(|n| *n == "open").count(), 1, "{seen:?}");
    assert_eq!(
        seen.iter().filter(|n| *n == "gamemode").count(),
        1,
        "{seen:?}"
    );
    assert!(
        !has(&seen, "secret"),
        "the proxy intercepts /secret, so the backend's node must go too: {seen:?}"
    );

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(backend_root_with_a_proxy_name_appears_once; p764 = 764, p774 = 774);

async fn forbidden_command_is_denied_and_not_forwarded(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;

    session.command("secret now").await.unwrap();
    let denial = session.expect_system_text(T).await.unwrap();
    assert!(denial.contains("permission"), "{denial}");

    session.command("vanilla x").await.unwrap();
    wait_forwarded(&mut conn, version, "vanilla x").await;

    assert_eq!(forwarded_commands(&conn, version), ["vanilla x"]);
    assert!(world.seen().is_empty(), "the forbidden handler ran");

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(forbidden_command_is_denied_and_not_forwarded; p47 = 47, p764 = 764, p774 = 774);

async fn every_label_runs_the_command_as_the_player(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, mut conn) = world.join(version).await;

    for line in ["open a b", "opn", "tester:open c", "quiet"] {
        session.command(line).await.unwrap();
        let reply = session.expect_system_text(T).await.unwrap();
        assert!(reply.starts_with("ran "), "{line}: {reply}");
    }
    session.command("vanilla").await.unwrap();
    wait_forwarded(&mut conn, version, "vanilla").await;

    assert_eq!(
        world.seen(),
        [
            "open Steve open [a,b]",
            "open Steve opn []",
            "open Steve tester:open [c]",
            "quiet Steve quiet []",
        ]
    );
    assert_eq!(forwarded_commands(&conn, version), ["vanilla"]);

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(every_label_runs_the_command_as_the_player; p47 = 47, p764 = 764, p774 = 774);

async fn tab_completion_carries_the_player_and_tooltips(version: ProtocolVersion) {
    let world = World::start().await;
    let (mut session, _conn) = world.join(version).await;

    session
        .send_packet(&STabCompleteRequest {
            transaction_id: 9,
            text: "/opn wo".into(),
        })
        .await
        .unwrap();
    let response = session.expect::<CTabCompleteResponse>(T).await.unwrap();

    assert_eq!(response.transaction_id, 9);
    assert_eq!((response.start, response.length), (5, 2));
    assert_eq!(response.matches.len(), 1);
    assert_eq!(response.matches[0].text, "world");
    let tooltip = response.matches[0].tooltip.as_deref().expect("a tooltip");
    assert_eq!(component_text(tooltip, version), "for Steve");

    session
        .send_packet(&STabCompleteRequest {
            transaction_id: 10,
            text: "/secret wo".into(),
        })
        .await
        .unwrap();
    let hidden = session.expect::<CTabCompleteResponse>(T).await.unwrap();
    assert_eq!(hidden.transaction_id, 10);
    assert!(hidden.matches.is_empty(), "{hidden:?}");

    world.proxy.shutdown().await.unwrap();
}

version_matrix!(tab_completion_carries_the_player_and_tooltips; p764 = 764, p774 = 774);

struct RecordingLimbo {
    commands: mpsc::UnboundedSender<String>,
}

impl LimboHandler for RecordingLimbo {
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
        _args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        let _ = self.commands.send(command.to_string());
        Box::pin(async {})
    }
}

async fn limbo_commands_share_the_same_dispatch(version: ProtocolVersion) {
    let seen = Seen::default();
    let handle = Handle::default();
    let (tx, mut limbo_commands) = mpsc::unbounded_channel();
    let limbo = ScriptedPlugin::new("gate").on_enable(move |ctx| {
        ctx.register_limbo_handler(Box::new(RecordingLimbo {
            commands: tx.clone(),
        }))
        .expect("the limbo handler registers");
    });
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("hub")
                .unreachable()
                .limbo_handlers([HOLD]),
        )
        .plugin(tester(&seen, &handle))
        .plugin(limbo)
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

    session.command("secret").await.unwrap();
    let denial = session.expect_system_text(T).await.unwrap();
    assert!(denial.contains("permission"), "{denial}");
    session.command("opn x").await.unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "ran open");
    session.command("login pw").await.unwrap();

    let reached = tokio::time::timeout(T, limbo_commands.recv())
        .await
        .expect("the limbo handler saw a command")
        .unwrap();
    assert_eq!(
        reached, "login",
        "proxy commands must not reach the limbo handler"
    );
    assert_eq!(*seen.lock().unwrap(), ["open Steve opn [x]"]);

    proxy.shutdown().await.unwrap();
}

version_matrix!(limbo_commands_share_the_same_dispatch; p47 = 47, p764 = 764, p774 = 774);

fn console(proxy: &TestProxy) -> ConsoleServices {
    let services = proxy.services();
    let running = proxy.running().expect("the proxy is running");
    ConsoleServices::new(
        Arc::clone(&services.player_registry),
        Arc::clone(&services.connection_registry),
        Arc::clone(&services.ban_manager),
        services.server_manager.clone(),
        Arc::new(ConfigServiceImpl::new(
            Arc::clone(&services.domain_router),
            services.config_path.clone(),
            Arc::clone(&services.config),
        )),
        Arc::clone(running.plugin_manager()),
        Arc::clone(&services.permission_service),
        Arc::clone(&services.command_manager),
        proxy.shutdown_token().clone(),
        running.start_time(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_console_runs_plugin_commands() {
    let world = World::start().await;
    let services = console(&world.proxy);
    let mut dispatcher = CommandDispatcher::new();
    register_all(&mut dispatcher);

    for line in ["tester:open a b", "/opn", "secret", "quiet"] {
        let output = dispatcher.dispatch(line, &services).await;
        assert!(matches!(output, CommandOutput::None), "{line}");
    }
    match dispatcher.dispatch("nope", &services).await {
        CommandOutput::Error(message) => assert!(message.contains("Unknown command"), "{message}"),
        _ => panic!("an unknown console command must be reported"),
    }

    assert_eq!(
        world.seen(),
        [
            "open Console tester:open [a,b]",
            "open Console opn []",
            "secret Console secret []",
            "quiet Console quiet []",
        ]
    );

    world.proxy.shutdown().await.unwrap();
}
