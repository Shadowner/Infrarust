#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::lifecycle::PreLoginEvent;
use infrarust_api::loader::PluginLoader;
use infrarust_api::plugin::{Plugin, PluginHealth, PluginRuntimeStatus, PluginState};
use infrarust_api::services::plugin_registry::PluginRegistry;
use infrarust_api::types::ProtocolVersion;
use infrarust_core::plugin::PluginRegistryImpl;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_loader_wasm::WasmPluginLoader;

use support::{
    EnvOptions, TestEnv, load_enabled, loader_from_toml, make_env_with, nil_profile, stage,
    write_script,
};

const SCRIPTED: &str = "scripted";
const PROMPTLY: Duration = Duration::from_secs(10);
const QUARANTINE_AFTER_TWO: &str =
    "[plugins.scripted.wasm.recovery]\nmax_restarts = 2\nbackoff_initial = \"30s\"\n";
const PAST_THE_BACKOFF: Duration = Duration::from_secs(31);

struct Enabled {
    _tmp: tempfile::TempDir,
    _data: PathBuf,
    env: TestEnv,
    _loader: WasmPluginLoader,
    plugin: Arc<dyn Plugin>,
}

impl Enabled {
    fn status(&self) -> PluginRuntimeStatus {
        self.plugin
            .runtime_status()
            .expect("a wasm plugin reports its runtime status")
    }
}

async fn enable_scripted(script: &str, proxy_toml: &str) -> Enabled {
    let (tmp, plugins_dir) = stage(SCRIPTED);
    write_script(&plugins_dir, SCRIPTED, script);
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().grant(SCRIPTED, "chat-intercept"),
    );
    let loader = loader_from_toml(proxy_toml);
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = Arc::from(load_enabled(&loader, &env.factory, SCRIPTED).await);
    Enabled {
        _tmp: tmp,
        _data: plugins_dir.join(SCRIPTED),
        env,
        _loader: loader,
        plugin,
    }
}

fn pre_login() -> PreLoginEvent {
    PreLoginEvent::new(
        nil_profile("Steve"),
        "127.0.0.1:25565".parse().unwrap(),
        ProtocolVersion::MINECRAFT_1_21,
        "play.example.com".to_owned(),
    )
}

fn chat() -> ChatMessageEvent {
    ChatMessageEvent::new(
        support::session_player(
            1,
            nil_profile("Steve"),
            ProtocolVersion::MINECRAFT_1_21.raw(),
            "127.0.0.1:40000".parse().unwrap(),
        ),
        "hello".to_owned(),
        false,
        None,
    )
}

async fn dispatch(env: &TestEnv, line: &str) -> bool {
    let outcome = tokio::time::timeout(
        PROMPTLY,
        env.command_manager.dispatch(support::console(), line),
    )
    .await
    .expect("a command returns promptly");
    outcome == DispatchOutcome::Executed
}

async fn let_the_backoff_pass() {
    tokio::time::pause();
    tokio::time::advance(PAST_THE_BACKOFF).await;
    tokio::time::resume();
}

#[tokio::test(flavor = "current_thread")]
async fn a_quarantined_plugin_reports_its_health_until_its_next_attempt() {
    let fx = enable_scripted(
        "on pre-login normal panic\ncmd greet record",
        QUARANTINE_AFTER_TWO,
    )
    .await;

    let fresh = fx.status();
    assert_eq!(fresh.health, PluginHealth::Healthy);
    assert_eq!(fresh.generation, 1);
    assert_eq!(fresh.restarts.in_window, 0);
    assert_eq!(fresh.restarts.max, 2);
    assert_eq!(fresh.last_fault, None);

    fx.env.event_bus.fire(pre_login()).await;
    let recovered = fx.status();
    assert_eq!(recovered.health, PluginHealth::Healthy);
    assert_eq!(recovered.generation, 2, "one fresh instance");
    assert_eq!(recovered.restarts.in_window, 1);
    let fault = recovered.last_fault.expect("the fault is kept");
    assert!(
        fault.cause.starts_with("the guest trapped: panicked at"),
        "{fault:?}"
    );
    assert_eq!(fault.generation, 1, "the instance that faulted");

    for _ in 0..2 {
        fx.env.event_bus.fire(pre_login()).await;
    }
    let quarantined = fx.status();
    let PluginHealth::Quarantined { retry_in } = quarantined.health else {
        panic!("the third fault in the window quarantines the plugin: {quarantined:?}");
    };
    assert!(
        retry_in > Duration::from_secs(20) && retry_in <= Duration::from_secs(30),
        "{retry_in:?}"
    );
    assert_eq!(quarantined.health.retry_in(), Some(retry_in));
    assert_eq!(quarantined.generation, 3);
    assert_eq!(quarantined.restarts.in_window, 2);
    assert_eq!(quarantined.last_fault.expect("kept").generation, 3);

    let_the_backoff_pass().await;
    assert!(dispatch(&fx.env, "greet").await);
    let back = fx.status();
    assert_eq!(back.health, PluginHealth::Healthy);
    assert_eq!(back.generation, 4, "the retry after the backoff");
    assert_eq!(back.restarts.in_window, 3);
}

#[tokio::test(flavor = "current_thread")]
async fn the_registry_carries_the_runtime_status_the_admin_api_and_console_read() {
    let fx = enable_scripted(
        "on pre-login normal panic",
        "[plugins.scripted.wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"1h\"\nbackoff_max = \"1h\"\n",
    )
    .await;
    let registry = PluginRegistryImpl::new();
    registry.insert_enabled(&fx.plugin.metadata(), &fx.plugin);

    fx.env.event_bus.fire(pre_login()).await;

    let info = registry.plugin_info(SCRIPTED).expect("listed");
    assert_eq!(
        info.state,
        PluginState::Enabled,
        "the lifecycle state stays enabled"
    );
    let runtime = info.runtime.expect("a wasm plugin has a runtime status");
    assert!(
        matches!(runtime.health, PluginHealth::Quarantined { .. }),
        "{runtime:?}"
    );
    assert_eq!(runtime.health.as_str(), "quarantined");
    assert_eq!(runtime.restarts.max, 0);
    let listed = registry.list_plugin_info()[0]
        .runtime
        .clone()
        .expect("listed with its runtime status");
    assert_eq!(listed.health.as_str(), "quarantined");
    assert_eq!(listed.generation, runtime.generation);
}

#[tokio::test(flavor = "current_thread")]
async fn a_disabled_plugin_reports_it_is_stopped() {
    let fx = enable_scripted("cmd greet record", "").await;
    fx.plugin.on_disable().await.unwrap();
    assert_eq!(fx.status().health, PluginHealth::Stopped);
    assert_eq!(fx.status().queue.depth, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_listener_shows_in_the_queue_depth_and_the_time_calls_waited() {
    const EVENTS: usize = 8;
    const HANDLER: Duration = Duration::from_millis(100);

    let fx = enable_scripted("on chat-message normal sleep 100", "").await;
    let idle = fx.status();
    assert_eq!(idle.queue.depth, 0);
    assert_eq!(idle.queue.capacity, 1024);
    assert_eq!(idle.queue.recent.span, Duration::from_secs(60));

    let bus = Arc::clone(&fx.env.event_bus);
    let fired: Vec<_> = (0..EVENTS)
        .map(|_| {
            let bus = Arc::clone(&bus);
            tokio::spawn(async move { bus.fire(chat()).await })
        })
        .collect();

    let started = Instant::now();
    let mut deepest = 0;
    while started.elapsed() < PROMPTLY && fired.iter().any(|task| !task.is_finished()) {
        deepest = deepest.max(fx.status().queue.depth);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    for task in fired {
        task.await.unwrap();
    }
    assert!(
        deepest >= EVENTS / 2,
        "the events waited in the queue behind the slow listener: at most {deepest} seen"
    );

    let recent = fx.status().queue.recent;
    assert_eq!(recent.taken, 1 + EVENTS as u64, "on_enable and every event");
    assert!(recent.peak_depth >= EVENTS / 2, "{recent:?}");
    let last_in_line = HANDLER * (EVENTS as u32 - 2);
    assert!(
        recent.wait_max >= last_in_line && recent.wait_p99 >= last_in_line,
        "the last event waited for the ones before it: {recent:?}"
    );
    assert!(
        recent.wait_p50 >= HANDLER && recent.wait_p50 <= recent.wait_p99,
        "{recent:?}"
    );
    assert_eq!(fx.status().queue.depth, 0, "drained");
}
