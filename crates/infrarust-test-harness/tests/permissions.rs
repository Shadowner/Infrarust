#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::event::{BoxFuture, EventPriority, ResultedEvent};
use infrarust_api::events::lifecycle::{PermissionsSetupEvent, PermissionsSetupResult};
use infrarust_api::permissions::{
    PermissionChecker, PermissionDefault, PermissionMap, PermissionNode, PermissionProvider,
    PermissionProviderRejected, PermissionSubject,
};
use infrarust_api::player::Player;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_core::console::ConsoleServices;
use infrarust_core::console::commands::register_all;
use infrarust_core::console::dispatcher::CommandDispatcher;
use infrarust_core::console::output::CommandOutput;
use infrarust_core::services::config_service::ConfigServiceImpl;
use infrarust_protocol::packets::play::chat::CSystemChatMessage;
use infrarust_protocol::packets::play::commands::{CCommands, CommandNode};
use infrarust_test_harness::text::component_text;
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, FakeBackend, FakeSessionServer, ProtocolVersion,
    ScriptedPlugin, ServerSpec, TestProxy, wire,
};
use toml::{Table, Value};

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion(774);
const PERMS: &str = "perms";
const DEMO_NODE: &str = "demo.use";

type Seen = Arc<Mutex<Vec<String>>>;

fn empty_tree() -> CCommands {
    CCommands {
        nodes: vec![CommandNode {
            flags: 0,
            children: vec![],
            redirect_node: None,
            name: None,
            parser: None,
            suggestions_type: None,
        }],
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

fn has_root(tree: &CCommands, name: &str) -> bool {
    roots(tree).iter().any(|root| root == name)
}

fn ir_children(tree: &CCommands) -> Vec<String> {
    let Some(ir) = tree
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("infrarust"))
    else {
        return Vec::new();
    };
    let mut names: Vec<String> = ir
        .children
        .iter()
        .filter_map(|&i| tree.nodes[i as usize].name.clone())
        .collect();
    names.sort_unstable();
    names
}

async fn join(
    proxy: &TestProxy,
    backend: &FakeBackend,
    name: &str,
) -> (ClientSession, BackendConn) {
    let session = proxy
        .client(VERSION)
        .login(name)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player(name, T).await.unwrap();
    (session, conn)
}

async fn text_containing(session: &mut ClientSession, needle: &str) -> String {
    let deadline = tokio::time::Instant::now() + T;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let text = session
            .expect_system_text(left)
            .await
            .unwrap_or_else(|e| panic!("no message containing {needle:?}: {e}"));
        if text.contains(needle) {
            return text;
        }
    }
}

async fn first_tree(session: &mut ClientSession, conn: &mut BackendConn) -> CCommands {
    conn.send_packet(&empty_tree()).await.unwrap();
    session.expect::<CCommands>(T).await.unwrap()
}

fn player(proxy: &TestProxy, name: &str) -> Arc<dyn Player> {
    proxy
        .services()
        .player_registry
        .get_player(name)
        .expect("the player is online")
}

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

async fn run_console(proxy: &TestProxy, line: &str) -> CommandOutput {
    let services = console(proxy);
    let mut dispatcher = CommandDispatcher::new();
    register_all(&mut dispatcher);
    dispatcher.dispatch(line, &services).await
}

fn permissions_table(entries: &[(&str, Value)]) -> impl FnOnce(&mut Table) + Send + use<> {
    let entries: Vec<(String, Value)> = entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect();
    move |table: &mut Table| {
        table.insert(
            "permissions".into(),
            Value::Table(entries.into_iter().collect()),
        );
    }
}

fn strings(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|v| Value::String((*v).to_string()))
            .collect(),
    )
}

fn select_perms() -> impl FnOnce(&mut Table) + Send + 'static {
    permissions_table(&[("provider", Value::String(PERMS.into()))])
}

#[derive(Default)]
struct Perms {
    players: Mutex<HashMap<String, PermissionMap>>,
    console: Mutex<PermissionMap>,
    subjects: Mutex<Vec<PermissionSubject>>,
}

impl Perms {
    fn grant(&self, player: &str, node: &str, value: bool) {
        self.players
            .lock()
            .unwrap()
            .entry(player.to_string())
            .or_default()
            .set(node, value);
    }

    fn grant_console(&self, node: &str, value: bool) {
        self.console.lock().unwrap().set(node, value);
    }

    fn subjects(&self) -> Vec<PermissionSubject> {
        self.subjects.lock().unwrap().clone()
    }
}

struct Provider(Arc<Perms>);

impl PermissionProvider for Provider {
    fn create_checker<'a>(
        &'a self,
        subject: &'a PermissionSubject,
    ) -> BoxFuture<'a, Arc<dyn PermissionChecker>> {
        self.0.subjects.lock().unwrap().push(subject.clone());
        let checker = match subject.profile() {
            Some(profile) => self
                .0
                .players
                .lock()
                .unwrap()
                .get(&profile.username)
                .cloned()
                .unwrap_or_default(),
            None => self.0.console.lock().unwrap().clone(),
        };
        Box::pin(async move { Arc::new(checker) as Arc<dyn PermissionChecker> })
    }
}

struct Probe {
    seen: Seen,
}

impl CommandHandler for Probe {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        self.seen
            .lock()
            .unwrap()
            .push(format!("{} {}", ctx.label, ctx.source.name()));
        ctx.source
            .send_message(Component::text(format!("ran {}", ctx.label)));
        Box::pin(async {})
    }
}

fn demo_commands(ctx: &dyn infrarust_api::plugin::PluginContext, seen: &Seen) {
    ctx.register_permission_node(
        PermissionNode::new(DEMO_NODE, PermissionDefault::False).description("Run /demo"),
    )
    .unwrap();
    ctx.command_manager()
        .register(
            CommandSpec::new("demo").permission(DEMO_NODE),
            Box::new(Probe {
                seen: Arc::clone(seen),
            }),
        )
        .unwrap();
}

fn perms_plugin(perms: &Arc<Perms>, seen: &Seen) -> ScriptedPlugin {
    let perms = Arc::clone(perms);
    let seen = Arc::clone(seen);
    ScriptedPlugin::new(PERMS).on_enable(move |ctx| {
        demo_commands(ctx, &seen);
        ctx.register_permission_provider(Arc::new(Provider(Arc::clone(&perms))))
            .unwrap();
    })
}

fn seen(seen: &Seen) -> Vec<String> {
    std::mem::take(&mut *seen.lock().unwrap())
}

async fn start(
    backend: &FakeBackend,
    plugins: Vec<ScriptedPlugin>,
    config: impl FnOnce(&mut Table) + Send + 'static,
) -> TestProxy {
    let mut builder = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(config);
    for plugin in plugins {
        builder = builder.plugin(plugin);
    }
    builder.start().await.unwrap()
}

async fn assert_no_tree_before(session: &mut ClientSession, marker: &str) {
    loop {
        let frame = session.recv_frame(T).await.unwrap();
        assert!(
            !wire::is::<CCommands>(&frame, VERSION),
            "this player's command tree must not be resent"
        );
        if wire::is::<CSystemChatMessage>(&frame, VERSION) {
            let message = wire::decode::<CSystemChatMessage>(&frame, VERSION).unwrap();
            if component_text(&message.content, VERSION).contains(marker) {
                return;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_selected_provider_decides_who_runs_a_command() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let perms = Arc::new(Perms::default());
    perms.grant("Steve", DEMO_NODE, true);
    let calls = Seen::default();
    let proxy = start(&backend, vec![perms_plugin(&perms, &calls)], select_perms()).await;
    let (mut steve, mut steve_conn) = join(&proxy, &backend, "Steve").await;
    let (mut alex, mut alex_conn) = join(&proxy, &backend, "Alex").await;

    let steve_tree = first_tree(&mut steve, &mut steve_conn).await;
    let alex_tree = first_tree(&mut alex, &mut alex_conn).await;
    assert!(has_root(&steve_tree, "demo"), "{:?}", roots(&steve_tree));
    assert!(!has_root(&alex_tree, "demo"), "{:?}", roots(&alex_tree));

    steve.command("demo").await.unwrap();
    assert_eq!(steve.expect_system_text(T).await.unwrap(), "ran demo");
    alex.command("demo").await.unwrap();
    let denial = alex.expect_system_text(T).await.unwrap();
    assert!(denial.contains("permission"), "{denial}");

    assert_eq!(seen(&calls), ["demo Steve"]);
    let steve_subject = perms
        .subjects()
        .into_iter()
        .find(|subject| subject.profile().is_some_and(|p| p.username == "Steve"))
        .expect("the provider was asked about Steve");
    assert_eq!(steve_subject.virtual_host(), Some("lobby.test"));
    assert!(!steve_subject.is_online_mode());
    assert_eq!(
        steve_subject.player_id(),
        Some(player(&proxy, "Steve").id())
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_permissions_resends_only_that_players_tree() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let perms = Arc::new(Perms::default());
    perms.grant("Steve", DEMO_NODE, true);
    let calls = Seen::default();
    let proxy = start(&backend, vec![perms_plugin(&perms, &calls)], select_perms()).await;
    let (mut steve, mut steve_conn) = join(&proxy, &backend, "Steve").await;
    let (mut alex, mut alex_conn) = join(&proxy, &backend, "Alex").await;
    assert!(has_root(
        &first_tree(&mut steve, &mut steve_conn).await,
        "demo"
    ));
    first_tree(&mut alex, &mut alex_conn).await;

    perms.grant("Steve", DEMO_NODE, false);
    steve.command("demo").await.unwrap();
    assert_eq!(
        steve.expect_system_text(T).await.unwrap(),
        "ran demo",
        "the provider's answer only applies once the player is refreshed"
    );

    player(&proxy, "Steve").refresh_permissions().await;

    let revoked = steve.expect::<CCommands>(T).await.unwrap();
    assert!(!has_root(&revoked, "demo"), "{:?}", roots(&revoked));
    steve.command("demo").await.unwrap();
    let denial = steve.expect_system_text(T).await.unwrap();
    assert!(denial.contains("permission"), "{denial}");

    player(&proxy, "Alex")
        .send_message(Component::text("marker"))
        .unwrap();
    assert_no_tree_before(&mut alex, "marker").await;

    perms.grant("Steve", DEMO_NODE, true);
    player(&proxy, "Steve").refresh_permissions().await;
    let restored = steve.expect::<CCommands>(T).await.unwrap();
    assert!(has_root(&restored, "demo"), "{:?}", roots(&restored));
    steve.command("demo").await.unwrap();
    assert_eq!(steve.expect_system_text(T).await.unwrap(), "ran demo");

    assert_eq!(seen(&calls), ["demo Steve", "demo Steve"]);
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_provider_leaves_everyone_with_the_node_defaults() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let calls = Seen::default();
    let commands = {
        let calls = Arc::clone(&calls);
        ScriptedPlugin::new("demo").on_enable(move |ctx| {
            ctx.register_permission_node(PermissionNode::new("demo.open", PermissionDefault::True))
                .unwrap();
            ctx.command_manager()
                .register(
                    CommandSpec::new("open").permission("demo.open"),
                    Box::new(Probe {
                        seen: Arc::clone(&calls),
                    }),
                )
                .unwrap();
            demo_commands(ctx, &calls);
        })
    };
    let proxy = start(
        &backend,
        vec![commands],
        permissions_table(&[
            ("provider", Value::String(PERMS.into())),
            ("player_commands", strings(&["list"])),
            ("trust_offline_admins", Value::Boolean(true)),
            ("admins", strings(&[&offline_uuid("Steve").to_string()])),
        ]),
    )
    .await;
    let (mut steve, mut conn) = join(&proxy, &backend, "Steve").await;

    let tree = first_tree(&mut steve, &mut conn).await;
    assert!(has_root(&tree, "open"), "{:?}", roots(&tree));
    assert!(!has_root(&tree, "demo"), "{:?}", roots(&tree));
    assert_eq!(ir_children(&tree), ["list"]);

    steve.command("open").await.unwrap();
    assert_eq!(steve.expect_system_text(T).await.unwrap(), "ran open");
    for denied in ["demo", "ir kick Alex"] {
        steve.command(denied).await.unwrap();
        let reply = steve.expect_system_text(T).await.unwrap();
        assert!(reply.contains("permission"), "{denied}: {reply}");
    }
    assert_eq!(seen(&calls), ["open Steve"]);
    assert!(!player(&proxy, "Steve").has_permission("infrarust.admin"));

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_console_runs_commands_under_its_own_checker() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let perms = Arc::new(Perms::default());
    perms.grant_console(DEMO_NODE, false);
    let calls = Seen::default();
    let proxy = start(&backend, vec![perms_plugin(&perms, &calls)], select_perms()).await;

    match run_console(&proxy, "demo").await {
        CommandOutput::Error(message) => assert!(message.contains("may not run"), "{message}"),
        _ => panic!("the provider denied demo.use to the console"),
    }
    assert!(seen(&calls).is_empty());
    assert!(perms.subjects().iter().any(PermissionSubject::is_console));

    perms.grant_console(DEMO_NODE, true);
    assert!(matches!(
        run_console(&proxy, "demo").await,
        CommandOutput::None
    ));
    assert_eq!(seen(&calls), ["demo Console"]);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_permissions_setup_override_still_beats_the_provider() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let perms = Arc::new(Perms::default());
    let calls = Seen::default();
    let override_alex =
        ScriptedPlugin::new("vip").on::<PermissionsSetupEvent>(EventPriority::NORMAL, |event| {
            if event.profile().username == "Alex" {
                event.set_result(PermissionsSetupResult::Custom(Arc::new(
                    PermissionMap::new().with(DEMO_NODE, true),
                )));
            }
        });
    let proxy = start(
        &backend,
        vec![perms_plugin(&perms, &calls), override_alex],
        select_perms(),
    )
    .await;
    let (mut alex, mut conn) = join(&proxy, &backend, "Alex").await;

    assert!(has_root(&first_tree(&mut alex, &mut conn).await, "demo"));
    alex.command("demo").await.unwrap();
    assert_eq!(alex.expect_system_text(T).await.unwrap(), "ran demo");

    player(&proxy, "Alex").refresh_permissions().await;
    assert!(has_root(
        &alex.expect::<CCommands>(T).await.unwrap(),
        "demo"
    ));
    assert!(player(&proxy, "Alex").has_permission(DEMO_NODE));

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_selected_and_capable_plugin_becomes_the_provider() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let attempt = |id: &'static str| {
        let outcomes = Arc::clone(&outcomes);
        ScriptedPlugin::new(id).on_enable(move |ctx| {
            let result = ctx.register_permission_provider(Arc::new(Provider(Arc::default())));
            outcomes.lock().unwrap().push((id, result));
        })
    };
    let proxy = start(
        &backend,
        vec![attempt("intruder"), attempt(PERMS)],
        |table: &mut Table| {
            select_perms()(table);
            let mut plugins = Table::new();
            plugins.insert(
                PERMS.into(),
                Value::Table(Table::from_iter([(
                    "deny".to_string(),
                    strings(&["permission-provider"]),
                )])),
            );
            table.insert("plugins".into(), Value::Table(plugins));
        },
    )
    .await;

    let mut outcomes = outcomes.lock().unwrap().clone();
    outcomes.sort_by_key(|(id, _)| *id);
    assert_eq!(
        outcomes,
        [
            (
                "intruder",
                Err(PermissionProviderRejected::NotSelected {
                    selected: PERMS.into()
                })
            ),
            (PERMS, Err(PermissionProviderRejected::MissingCapability)),
        ]
    );

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passthrough_players_get_the_providers_checker() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let perms = Arc::new(Perms::default());
    perms.grant("Steve", DEMO_NODE, true);
    let calls = Seen::default();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .plugin(perms_plugin(&perms, &calls))
        .patch_config(select_perms())
        .start()
        .await
        .unwrap();
    let _steve = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();

    let steve = player(&proxy, "Steve");
    assert!(steve.has_permission(DEMO_NODE));
    let subject = perms
        .subjects()
        .into_iter()
        .find(|subject| subject.player_id() == Some(steve.id()))
        .expect("the provider was asked about the passthrough player");
    assert_eq!(subject.virtual_host(), Some("lobby.test"));
    assert!(!subject.is_online_mode());

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_ir_node_opens_that_subcommand_only() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let grants =
        ScriptedPlugin::new("grants").on::<PermissionsSetupEvent>(EventPriority::NORMAL, |event| {
            event.set_result(PermissionsSetupResult::Custom(Arc::new(
                PermissionMap::new().with("infrarust.command.list", true),
            )));
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(grants)
        .start()
        .await
        .unwrap();
    let (mut steve, mut conn) = join(&proxy, &backend, "Steve").await;

    let tree = first_tree(&mut steve, &mut conn).await;
    assert_eq!(ir_children(&tree), ["list"], "{:?}", roots(&tree));

    steve.command("ir list").await.unwrap();
    text_containing(&mut steve, "Servers (1)").await;
    text_containing(&mut steve, "lobby").await;
    steve.command("ir kick Alex").await.unwrap();
    let denial = steve.expect_system_text(T).await.unwrap();
    assert!(denial.contains("permission"), "{denial}");

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn op_sends_the_online_player_a_new_tree() {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .start()
        .await
        .unwrap();
    let (mut steve, mut conn) = join(&proxy, &backend, "Steve").await;
    let before = first_tree(&mut steve, &mut conn).await;
    assert!(!roots(&before).contains(&"infrarust".to_string()));

    let output = run_console(&proxy, "op Steve").await;
    assert!(matches!(output, CommandOutput::Success(_)));
    assert_eq!(
        proxy.services().permission_service.admin_list(),
        [offline_uuid("Steve")]
    );

    let after = steve.expect::<CCommands>(T).await.unwrap();
    assert!(
        roots(&after).contains(&"infrarust".to_string()),
        "{:?}",
        roots(&after)
    );
    assert!(ir_children(&after).contains(&"kick".to_string()));

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn players_keep_the_builtin_ir_gates() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(permissions_table(&[
            ("player_commands", strings(&["list", "kick"])),
            ("admins", strings(&[&offline_uuid("Steve").to_string()])),
        ]))
        .start()
        .await
        .unwrap();
    let (mut steve, mut conn) = join(&proxy, &backend, "Steve").await;

    let tree = first_tree(&mut steve, &mut conn).await;
    assert_eq!(ir_children(&tree), ["list"]);
    steve.command("ir list").await.unwrap();
    text_containing(&mut steve, "Servers (1)").await;
    text_containing(&mut steve, "lobby").await;
    for denied in ["ir kick Alex", "ir reload"] {
        steve.command(denied).await.unwrap();
        let reply = steve.expect_system_text(T).await.unwrap();
        assert!(reply.contains("permission"), "{denied}: {reply}");
    }

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn online_admins_get_every_subcommand() {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .patch_config(permissions_table(&[(
            "admins",
            strings(&[&offline_uuid("Steve").to_string()]),
        )]))
        .start()
        .await
        .unwrap();
    let (mut steve, mut conn) = join(&proxy, &backend, "Steve").await;

    let tree = first_tree(&mut steve, &mut conn).await;
    for name in ["kick", "list", "plugin", "reload", "send"] {
        assert!(ir_children(&tree).contains(&name.to_string()), "{name}");
    }
    steve.command("ir reload").await.unwrap();
    let reply = steve.expect_system_text(T).await.unwrap();
    assert!(reply.contains("auto-reloads"), "{reply}");

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_offline_admins_get_every_subcommand() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(permissions_table(&[
            ("trust_offline_admins", Value::Boolean(true)),
            ("admins", strings(&[&offline_uuid("Steve").to_string()])),
        ]))
        .start()
        .await
        .unwrap();
    let (mut steve, mut conn) = join(&proxy, &backend, "Steve").await;
    let (mut alex, mut alex_conn) = join(&proxy, &backend, "Alex").await;

    let tree = first_tree(&mut steve, &mut conn).await;
    assert!(ir_children(&tree).contains(&"reload".to_string()));
    assert!(ir_children(&first_tree(&mut alex, &mut alex_conn).await).is_empty());
    steve.command("ir reload").await.unwrap();
    let reply = steve.expect_system_text(T).await.unwrap();
    assert!(reply.contains("auto-reloads"), "{reply}");

    proxy.shutdown().await.unwrap();
}
