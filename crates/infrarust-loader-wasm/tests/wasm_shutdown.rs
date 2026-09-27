#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::types::ProtocolVersion;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::manager::{PluginManager, ShutdownLimits};
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
        while !log_lines(dir, id)
            .iter()
            .any(|line| line.starts_with(prefix))
        {
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

fn start_command(commands: &Arc<CommandManagerImpl>, line: impl Into<String>) {
    let commands = Arc::clone(commands);
    let line = line.into();
    tokio::spawn(async move { commands.dispatch(support::console(), &line).await });
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
    add_probe(
        &dir,
        "busy",
        "id=busy\ncmd=hold:sleep:600000\ndisable=sleep:600000\n",
    );
    let mut running = start(&dir, SHORT_CALLS).await;
    start_command(&running.commands, "hold");
    wait_for_line(&dir, "busy", "cmd hold start").await;
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!("blocked command plus sleeping on_disable: shutdown took {took:?}");
    assert!(took < MAX_CALL * 3, "{took:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_with_several_stuck_plugins_is_not_serialised_per_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let stuck = ["stuck-a", "stuck-b"];
    let slow = ["slow-c", "slow-d", "slow-e", "slow-f"];
    for id in stuck {
        add_probe(
            &dir,
            id,
            &format!("id={id}\ncmd={id}-hold:sleep:600000\ndisable=sleep:600000\n"),
        );
    }
    for id in slow {
        add_probe(&dir, id, &format!("id={id}\ndisable=sleep:1500\n"));
    }
    let mut running = start(&dir, SHORT_CALLS).await;
    for id in stuck {
        start_command(&running.commands, format!("{id}-hold"));
        wait_for_line(&dir, id, &format!("cmd {id}-hold start")).await;
    }
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!(
        "{} stuck plugins and {} whose on_disable takes 1.5 s: shutdown took {took:?}",
        stuck.len(),
        slow.len()
    );
    assert!(
        took < Duration::from_secs(3),
        "shutdown took {took:?}: one on_disable after the other takes at least 6 s"
    );
    for id in slow {
        assert!(
            log_lines(&dir, id)
                .iter()
                .any(|line| line.starts_with("disable-end")),
            "{id} ran its on_disable to the end: {:?}",
            log_lines(&dir, id)
        );
    }
    for id in stuck {
        let lines = log_lines(&dir, id);
        assert!(
            !lines.iter().any(|line| line.starts_with("disable ")),
            "{id}'s stuck call was cut when the shutdown started, so it has no instance to run on_disable in: {lines:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_on_disable_past_the_shutdown_limit_is_cut() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "sleepy", "id=sleepy\ndisable=sleep:600000\n");
    add_probe(&dir, "prompt", "id=prompt\ndisable=sleep:100\n");
    let mut running = start(&dir, "").await;
    running.manager.set_shutdown_limits(ShutdownLimits {
        on_disable: Duration::from_millis(500),
        total: Duration::from_secs(10),
    });
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!("on_disable sleeping past a 500 ms limit: shutdown took {took:?}");
    assert!(
        took < Duration::from_secs(2),
        "the default 60 s max_call_duration does not hold the shutdown: {took:?}"
    );
    assert!(
        log_lines(&dir, "prompt")
            .iter()
            .any(|line| line.starts_with("disable-end")),
        "{:?}",
        log_lines(&dir, "prompt")
    );
    assert!(
        !log_lines(&dir, "sleepy")
            .iter()
            .any(|line| line.starts_with("disable-end")),
        "{:?}",
        log_lines(&dir, "sleepy")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_whole_plugin_phase_ends_at_its_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "zz-base", "id=zz-base\ndisable=sleep:600000\n");
    add_probe(&dir, "mid", "id=mid\ndep=zz-base\ndisable=sleep:600000\n");
    add_probe(&dir, "aa-top", "id=aa-top\ndep=mid\ndisable=sleep:600000\n");
    let mut running = start(&dir, "").await;
    running.manager.set_shutdown_limits(ShutdownLimits {
        on_disable: Duration::from_secs(1),
        total: Duration::from_millis(1500),
    });
    let took = timed_shutdown(&mut running.manager).await;
    eprintln!(
        "three levels, each on_disable past its 1 s limit, 1.5 s in all: shutdown took {took:?}"
    );
    assert!(took < Duration::from_millis(2500), "{took:?}");
    assert!(
        log_lines(&dir, "mid")
            .iter()
            .any(|line| line.starts_with("disable ")),
        "the second level still starts its on_disable: {:?}",
        log_lines(&dir, "mid")
    );
}

fn record_panics_on(threads: &'static str) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current()
            .name()
            .is_some_and(|name| name.starts_with(threads))
        {
            record.lock().unwrap().push(info.to_string());
        }
        previous(info);
    }));
    seen
}

#[test]
fn a_plugin_cut_at_the_shutdown_limit_is_not_left_in_a_file_call_when_the_runtime_stops() {
    let panics = record_panics_on("flt09-");
    for round in 0..3 {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("flt09-worker")
            .enable_all()
            .build()
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        add_probe(&dir, "writer", "id=writer\ndisable=write\n");
        let took = runtime.block_on(async {
            let mut running = start(&dir, "[wasm]\ncpu_budget = \"60s\"\n").await;
            running.manager.set_shutdown_limits(ShutdownLimits {
                on_disable: Duration::from_millis(300),
                total: Duration::from_secs(5),
            });
            timed_shutdown(&mut running.manager).await
        });
        drop(runtime);
        eprintln!("round {round}: on_disable writing in a loop, shutdown took {took:?}");
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert!(
            log_lines(&dir, "writer")
                .iter()
                .any(|line| line.starts_with("write ")),
            "on_disable was writing when it was cut"
        );
    }
    let panics = panics.lock().unwrap().clone();
    assert!(
        panics.is_empty(),
        "the runtime stopped while a guest was still in a file call: {panics:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_of_a_quarantined_plugin_is_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(
        &dir,
        "faulty",
        "id=faulty\npost-login=panic\ndisable=sleep:600000\n",
    );
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
        !log_lines(&dir, "faulty")
            .iter()
            .any(|line| line.starts_with("disable ")),
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
