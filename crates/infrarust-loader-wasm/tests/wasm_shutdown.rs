#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::types::ProtocolVersion;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::manager::PluginManager;
use infrarust_core::services::command_manager::CommandManagerImpl;

use support::{fixture_path, loader_from_toml, make_env, nil_profile};

const MARKER: &[u8] = b"LIFPROBE-BLOB-V1";
const HARD_STOP: Duration = Duration::from_secs(120);
const SHORT_CALLS: &str = "[wasm]\nmax_call_duration = \"2s\"\n";
const MAX_CALL: Duration = Duration::from_secs(2);

fn add_probe(dir: &Path, stem: &str, settings: &str) {
    let mut bytes = std::fs::read(fixture_path("lif-probe")).unwrap();
    let at = bytes
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .unwrap()
        + MARKER.len();
    bytes[at..at + settings.len()].copy_from_slice(settings.as_bytes());
    bytes[at + settings.len()] = 0xFF;
    std::fs::write(dir.join(format!("{stem}.wasm")), bytes).unwrap();
}

struct Running {
    manager: PluginManager,
    event_bus: Arc<EventBusImpl>,
    commands: Arc<CommandManagerImpl>,
}

async fn start(dir: &Path, proxy_toml: &str) -> Running {
    let mut manager = PluginManager::new(vec![Box::new(loader_from_toml(proxy_toml))]);
    manager.discover_all(dir).await.unwrap();
    let env = make_env(dir.to_path_buf());
    let event_bus = Arc::clone(&env.event_bus);
    let commands = Arc::clone(&env.command_manager);
    let errors = manager.load_and_enable_all(Arc::new(env.factory)).await;
    assert!(errors.is_empty(), "{errors:?}");
    Running {
        manager,
        event_bus,
        commands,
    }
}

fn log_lines(dir: &Path, id: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(id).join("log.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

async fn wait_for_line(dir: &Path, id: &str, prefix: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !log_lines(dir, id).iter().any(|line| line.starts_with(prefix)) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{id} never logged `{prefix}`: {:?}", log_lines(dir, id)));
}

async fn timed_shutdown(manager: &mut PluginManager) -> Duration {
    let started = Instant::now();
    tokio::time::timeout(HARD_STOP, manager.shutdown())
        .await
        .expect("shutdown finished before the hard stop");
    started.elapsed()
}

fn start_command(commands: &Arc<CommandManagerImpl>, line: &'static str) {
    let commands = Arc::clone(commands);
    tokio::spawn(async move { commands.dispatch(support::console(), line).await });
}

fn post_login() -> PostLoginEvent {
    PostLoginEvent::new(support::session_player(
        1,
        nil_profile("Steve"),
        ProtocolVersion::MINECRAFT_1_21.raw(),
        "127.0.0.1:40000".parse().unwrap(),
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sleeping_on_disable_is_cut_at_max_call_duration() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "sleepy", "id=sleepy\ndisable=sleep:600000\n");
    let mut running = start(&dir, SHORT_CALLS).await;
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!("sleeping on_disable: shutdown took {took:?}");
    assert!(took < MAX_CALL * 3, "{took:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spinning_on_disable_is_cut_by_the_cpu_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "spinny", "id=spinny\ndisable=spin\n");
    let mut running = start(&dir, "[wasm]\ncpu_budget = \"500ms\"\n").await;
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!("spinning on_disable: shutdown took {took:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_while_a_command_is_blocked_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "busy", "id=busy\ncmd=hold:sleep:600000\ndisable=sleep:600000\n");
    let mut running = start(&dir, SHORT_CALLS).await;
    start_command(&running.commands, "hold");
    wait_for_line(&dir, "busy", "cmd hold start").await;
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!("blocked command plus sleeping on_disable: shutdown took {took:?}");
    assert!(took < MAX_CALL * 3, "{took:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-09: shutdown, unload and disable are unbounded"]
async fn shutdown_with_several_stuck_plugins_is_not_serialised_per_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let plugins = ["stuck-a", "stuck-b", "stuck-c", "stuck-d"];
    for id in plugins {
        add_probe(
            &dir,
            id,
            &format!("id={id}\ncmd={id}-hold:sleep:600000\ndisable=sleep:600000\n"),
        );
    }
    let mut running = start(&dir, SHORT_CALLS).await;
    for (id, line) in plugins
        .iter()
        .zip(["stuck-a-hold", "stuck-b-hold", "stuck-c-hold", "stuck-d-hold"])
    {
        start_command(&running.commands, line);
        wait_for_line(&dir, id, &format!("cmd {line} start")).await;
    }
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!(
        "{} stuck plugins, max_call_duration {MAX_CALL:?}: shutdown took {took:?}",
        plugins.len()
    );
    assert!(
        took < MAX_CALL * 3,
        "shutdown took {took:?}: each stuck plugin adds up to two max_call_duration, one after the other"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_of_a_quarantined_plugin_is_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "faulty", "id=faulty\npost-login=panic\ndisable=sleep:600000\n");
    let mut running = start(
        &dir,
        "[wasm]\nmax_call_duration = \"2s\"\n[wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"1h\"\nbackoff_max = \"1h\"\n",
    )
    .await;
    running.event_bus.fire(post_login()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!("quarantined plugin: shutdown took {took:?}");
    assert!(took < Duration::from_secs(1), "{took:?}");
    assert!(
        !log_lines(&dir, "faulty").iter().any(|line| line.starts_with("disable ")),
        "a quarantined plugin has no instance to run on_disable in"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_with_a_full_queue_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "clogged", "id=clogged\npost-login=sleep:600000\n");
    let mut running = start(
        &dir,
        "[wasm]\nmax_call_duration = \"2s\"\nqueue_capacity = 4\n",
    )
    .await;
    for _ in 0..16 {
        let bus = Arc::clone(&running.event_bus);
        tokio::spawn(async move { bus.fire(post_login()).await });
    }
    wait_for_line(&dir, "clogged", "post-login start").await;
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!("full queue: shutdown took {took:?}");
    assert!(took < MAX_CALL * 3, "{took:?}");
}
