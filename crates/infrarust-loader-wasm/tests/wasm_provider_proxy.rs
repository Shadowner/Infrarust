#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(all(feature = "wasm", wasm_fixtures_available))]

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::command::CommandSource;
use infrarust_api::permissions::AllPermissionsChecker;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_protocol::packets::play::commands::{CCommands, CommandNode};
use infrarust_test_harness::{
    BackendConn, ClientSession, ConnectionState, FakeBackend, ProtocolVersion, ServerSpec,
    TestProxy,
};
use toml::Value;
use toml::value::Table;

use support::{add_fixture, fresh_loader};

const T: Duration = infrarust_test_harness::DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion(774);
const PROVIDER: &str = "provider";

fn read_log(data: &Path) -> Vec<String> {
    std::fs::read_to_string(data.join("log.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn stage_provider(config: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    add_fixture(&plugins_dir, PROVIDER, PROVIDER);
    let data = plugins_dir.join(PROVIDER);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("provider.txt"), config).unwrap();
    (tmp, plugins_dir)
}

fn select_provider(
    plugins_dir: PathBuf,
    section: &'static str,
    capability: &'static str,
) -> impl FnOnce(&mut Table) + Send + 'static {
    move |table| {
        table.insert(
            "plugins_dir".into(),
            Value::String(plugins_dir.to_string_lossy().into_owned()),
        );
        let grants = Table::from_iter([(
            "permissions".to_owned(),
            Value::Array(vec![Value::String(capability.into())]),
        )]);
        table.insert(
            "plugins".into(),
            Value::Table(Table::from_iter([(
                PROVIDER.to_owned(),
                Value::Table(grants),
            )])),
        );
        let selection = table
            .entry(section)
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .expect("a table");
        selection.insert("provider".into(), Value::String(PROVIDER.into()));
    }
}

async fn start(
    backend: &FakeBackend,
    config: impl FnOnce(&mut Table) + Send + 'static,
) -> TestProxy {
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .loader(Box::new(fresh_loader()))
        .patch_config(config)
        .start()
        .await
        .unwrap();
    assert!(
        proxy.plugin_context(PROVIDER).await.is_some(),
        "the WASM provider is enabled"
    );
    proxy
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

async fn first_tree(session: &mut ClientSession, conn: &mut BackendConn) -> CCommands {
    conn.send_packet(&empty_tree()).await.unwrap();
    session.expect::<CCommands>(T).await.unwrap()
}

async fn console(proxy: &TestProxy, line: &str) {
    let outcome = proxy
        .services()
        .command_manager
        .dispatch(
            CommandSource::console(Arc::new(AllPermissionsChecker)),
            line,
        )
        .await;
    assert_eq!(outcome, DispatchOutcome::Executed, "{line}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wasm_ban_provider_selected_by_config_refuses_a_banned_login_with_its_message() {
    let (_tmp, plugins_dir) = stage_provider("bans\nban Banned griefing");
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = start(
        &backend,
        select_provider(plugins_dir.clone(), "ban", "ban-provider"),
    )
    .await;

    let refused = proxy
        .client(VERSION)
        .login("Banned")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(refused.state, ConnectionState::Login, "{refused:?}");
    assert_eq!(refused.text, "provider: griefing");
    assert_eq!(proxy.connection_count(), 0);

    let (steve, _conn) = join(&proxy, &backend, "Steve").await;
    assert_eq!(proxy.connection_count(), 1);
    assert_eq!(read_log(&plugins_dir.join(PROVIDER)), ["enable", "bans ok"]);

    steve.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wasm_permission_provider_decides_who_sees_and_runs_a_command() {
    let (_tmp, plugins_dir) =
        stage_provider("permissions\ngrant Steve demo.use true\ncommand demo demo.use");
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = start(
        &backend,
        select_provider(plugins_dir.clone(), "permissions", "permission-provider"),
    )
    .await;
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

    assert_eq!(
        read_log(&plugins_dir.join(PROVIDER)),
        ["enable", "permissions ok", "ran demo Steve"]
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_snapshot_changes_a_players_command_tree_live() {
    let (_tmp, plugins_dir) = stage_provider("permissions\ncommand demo demo.use");
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = start(
        &backend,
        select_provider(plugins_dir.clone(), "permissions", "permission-provider"),
    )
    .await;
    let (mut alex, mut alex_conn) = join(&proxy, &backend, "Alex").await;
    assert!(!has_root(
        &first_tree(&mut alex, &mut alex_conn).await,
        "demo"
    ));

    console(&proxy, "pset Alex demo.use true").await;
    let granted = alex.expect::<CCommands>(T).await.unwrap();
    assert!(has_root(&granted, "demo"), "{:?}", roots(&granted));
    alex.command("demo").await.unwrap();
    assert_eq!(alex.expect_system_text(T).await.unwrap(), "ran demo");

    console(&proxy, "pset Alex demo.use false").await;
    let revoked = alex.expect::<CCommands>(T).await.unwrap();
    assert!(!has_root(&revoked, "demo"), "{:?}", roots(&revoked));

    assert_eq!(
        read_log(&plugins_dir.join(PROVIDER)),
        [
            "enable",
            "permissions ok",
            "pset Alex ok",
            "ran demo Alex",
            "pset Alex ok",
        ]
    );
    proxy.shutdown().await.unwrap();
}
