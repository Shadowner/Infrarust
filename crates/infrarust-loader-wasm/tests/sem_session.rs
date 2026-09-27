#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(all(feature = "wasm", wasm_fixtures_available))]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::command::CommandSource;
use infrarust_api::permissions::AllPermissionsChecker;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_test_harness::{ClientSession, FakeBackend, ProtocolVersion, ServerSpec, TestProxy};
use tokio::time::Instant;
use toml::Value;
use toml::value::Table;

use support::{add_fixture, fresh_loader, read_log};

const T: Duration = infrarust_test_harness::DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion::V1_21;
const PROBE: &str = "sem-probe";

fn grant(plugins_dir: PathBuf, capabilities: &'static [&'static str]) -> impl FnOnce(&mut Table) + Send + 'static {
    move |table| {
        table.insert(
            "plugins_dir".into(),
            Value::String(plugins_dir.to_string_lossy().into_owned()),
        );
        let grants = Table::from_iter([(
            "permissions".to_owned(),
            Value::Array(
                capabilities
                    .iter()
                    .map(|capability| Value::String((*capability).into()))
                    .collect(),
            ),
        )]);
        table.insert(
            "plugins".into(),
            Value::Table(Table::from_iter([(PROBE.to_owned(), Value::Table(grants))])),
        );
    }
}

struct World {
    proxy: TestProxy,
    backend: FakeBackend,
    _dir: tempfile::TempDir,
    data: PathBuf,
}

impl World {
    async fn start(config: &str, limbo: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let plugins_dir = dir.path().to_path_buf();
        add_fixture(&plugins_dir, PROBE, PROBE);
        let data = plugins_dir.join(PROBE);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("probe.txt"), config).unwrap();
        let backend = FakeBackend::builder().spawn().await.unwrap();
        let mut lobby = ServerSpec::offline("lobby").backend(backend.addr());
        if limbo {
            lobby = lobby.limbo_handlers(["keeper"]);
        }
        let proxy = TestProxy::builder()
            .server(lobby)
            .loader(Box::new(fresh_loader()))
            .patch_config(grant(plugins_dir, &["limbo"]))
            .start()
            .await
            .unwrap();
        assert!(proxy.plugin_context(PROBE).await.is_some());
        Self {
            proxy,
            backend,
            _dir: dir,
            data,
        }
    }

    fn log(&self) -> Vec<String> {
        read_log(&self.data)
    }

    async fn wait_for(&self, line: &str) {
        let deadline = Instant::now() + T;
        while !self.log().iter().any(|seen| seen == line) {
            assert!(Instant::now() < deadline, "never logged {line:?}: {:?}", self.log());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_prefix(&self, prefix: &str, count: usize) -> Vec<String> {
        let deadline = Instant::now() + T;
        loop {
            let lines: Vec<String> = self
                .log()
                .into_iter()
                .filter(|line| line.starts_with(prefix))
                .collect();
            if lines.len() >= count {
                return lines;
            }
            assert!(
                Instant::now() < deadline,
                "expected {count} lines starting with {prefix:?}: {:?}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn console(&self, line: &str) {
        let outcome = self
            .proxy
            .services()
            .command_manager
            .dispatch(CommandSource::console(Arc::new(AllPermissionsChecker)), line)
            .await;
        assert_eq!(outcome, DispatchOutcome::Executed, "{line}");
    }

    async fn enter(&self, name: &str) -> (ClientSession, String) {
        let session = self
            .proxy
            .client(VERSION)
            .login(name)
            .await
            .unwrap()
            .joined()
            .unwrap();
        let entered = self.wait_for_prefix("enter ", 1).await;
        let id = entered.last().unwrap().trim_start_matches("enter ").to_owned();
        (session, id)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_player_released_from_a_later_command_reaches_the_backend() {
    let world = World::start("keeper", true).await;
    let (_session, id) = world.enter("Steve").await;
    world.console(&format!("hdone {id}")).await;
    world.wait_for(&format!("hdone {id} ok cancelled=false")).await;
    world
        .backend
        .next_connection(T)
        .await
        .expect("the released player reaches the backend");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_handle_kept_after_the_player_left_reports_player_gone_and_completing_it_is_a_no_op() {
    let world = World::start("keeper", true).await;
    let (session, id) = world.enter("Steve").await;
    session.quit().await;
    world.wait_for(&format!("ended {id}")).await;
    world.console(&format!("hsend {id}")).await;
    world.console(&format!("hdone {id}")).await;
    let sent = world.wait_for_prefix(&format!("hsend {id} "), 1).await;
    let done = world.wait_for_prefix(&format!("hdone {id} "), 1).await;
    assert_eq!(
        [sent[0].as_str(), done[0].as_str()],
        [
            format!("hsend {id} player-gone cancelled=true").as_str(),
            format!("hdone {id} ok cancelled=true").as_str()
        ],
        "{:?}",
        world.log()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_players_held_at_once_are_all_tracked_and_all_released_when_they_leave() {
    let world = World::start("keeper", true).await;
    let mut sessions = Vec::new();
    for index in 0..20 {
        let session = world
            .proxy
            .client(VERSION)
            .login(&format!("Held{index}"))
            .await
            .unwrap()
            .joined()
            .unwrap();
        sessions.push(session);
    }
    world.wait_for_prefix("enter ", 20).await;
    world.console("hcount").await;
    world.wait_for("hcount 20 cancelled=0").await;
    for session in sessions {
        session.quit().await;
    }
    world.wait_for_prefix("ended ", 20).await;
    world.console("hcount").await;
    world.wait_for("hcount 20 cancelled=20").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completing_a_hold_with_another_timed_hold_keeps_the_player_out_of_the_backend() {
    let world = World::start("keeper", true).await;
    let (_session, id) = world.enter("Steve").await;
    world.console(&format!("hrearm {id}")).await;
    let answered = world.wait_for_prefix(&format!("hrearm {id} "), 1).await;
    let reached = world.backend.next_connection(Duration::from_millis(800)).await.is_ok();
    assert!(
        !reached,
        "a gate that answers `complete(HoldWithTimeout {{ on_timeout: Deny }})` let the player through \
         to the backend; the guest was told {:?}",
        answered[0]
    );
}
