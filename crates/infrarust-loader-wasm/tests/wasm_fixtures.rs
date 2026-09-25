#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant};

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{ServerPreConnectEvent, ServerPreConnectResult};
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::types::{PlayerId, ProtocolVersion, ServerId};
use infrarust_core::event_bus::EventBusConfig;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_loader_wasm::WasmPluginLoader;
use tracing::instrument::WithSubscriber;

use support::mock_services::{
    CountingPlayerRegistry, Gate, GatedBanService, MapConfigService, MockPlayerRegistry,
    PanickingBanService, RecordingPlayerRegistry,
};
use support::{
    EnvOptions, TestEnv, add_fixture, fresh_loader, load_enabled, loader_from_toml, make_env,
    make_env_with, nil_profile, read_log, stage, write_script,
};

fn make_factory(plugins_dir: &Path) -> PluginContextFactoryImpl {
    make_env(plugins_dir.to_path_buf()).factory
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scripted_sdk_plugin_lifecycle() {
    let (_tmp, plugins_dir) = stage("scripted");
    write_script(&plugins_dir, "scripted", "");
    let loader = fresh_loader();
    let factory = make_factory(&plugins_dir);

    let metas = loader.discover(&plugins_dir).await.unwrap();
    assert!(
        metas.iter().any(|m| m.id == "scripted"),
        "the SDK-generated metadata export names the plugin"
    );

    let plugin = loader.load("scripted", &factory).await.expect("load");
    let ctx = factory.create_context("scripted");
    plugin.on_enable(ctx.as_ref()).await.expect("on_enable ok");
    let data_dir = plugins_dir.join("scripted");
    assert_eq!(
        read_log(&data_dir),
        ["enable"],
        "on_enable ran and wrote into its WASI-scoped data dir"
    );

    plugin.on_disable().await.expect("on_disable ok");
    assert_eq!(read_log(&data_dir), ["enable", "disable"]);
    loader.unload("scripted").await.expect("unload ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trap_on_purpose_is_contained() {
    let (_tmp, plugins_dir) = stage("scripted");
    add_fixture(&plugins_dir, "trap-on-purpose", "trap");
    write_script(&plugins_dir, "scripted", "");

    let loader = fresh_loader();
    let factory = make_factory(&plugins_dir);
    loader.discover(&plugins_dir).await.unwrap();

    let trap = loader
        .load("trap-on-purpose", &factory)
        .await
        .expect("load trap");
    let trap_ctx = factory.create_context("trap-on-purpose");
    assert!(
        trap.on_enable(trap_ctx.as_ref()).await.is_err(),
        "a guest trap must surface as Err, not Ok"
    );

    let scripted = loader
        .load("scripted", &factory)
        .await
        .expect("engine still usable after a trap");
    let scripted_ctx = factory.create_context("scripted");
    scripted
        .on_enable(scripted_ctx.as_ref())
        .await
        .expect("another plugin enables after a trap");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cpu_spin_interrupted_by_epoch() {
    let (_tmp, plugins_dir) = stage("cpu-spin");
    let loader = fresh_loader();
    let factory = make_factory(&plugins_dir);
    loader.discover(&plugins_dir).await.unwrap();

    let plugin = loader
        .load("cpu-spin", &factory)
        .await
        .expect("load cpu-spin");
    let ctx = factory.create_context("cpu-spin");

    let outcome =
        tokio::time::timeout(Duration::from_secs(10), plugin.on_enable(ctx.as_ref())).await;
    match outcome {
        Ok(result) => assert!(result.is_err(), "cpu-spin must trap (Err), not return Ok"),
        Err(_) => panic!("cpu-spin was not interrupted within 10s — epoch interruption broken"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_bomb_refused_by_limiter() {
    let (_tmp, plugins_dir) = stage("memory-bomb");
    let loader = fresh_loader();
    let factory = make_factory(&plugins_dir);
    loader.discover(&plugins_dir).await.unwrap();

    let plugin = loader
        .load("memory-bomb", &factory)
        .await
        .expect("load memory-bomb");
    let ctx = factory.create_context("memory-bomb");

    let outcome = tokio::time::timeout(Duration::from_secs(10), plugin.on_enable(ctx.as_ref()))
        .await
        .expect("memory-bomb should fail fast, not hang/OOM");
    assert!(
        outcome.is_err(),
        "memory-bomb must be refused by the resource limiter (Err)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_aot_cache_reused() {
    let (_tmp, plugins_dir) = stage("scripted");

    {
        let loader = fresh_loader();
        loader.discover(&plugins_dir).await.unwrap();
    }
    let cache_dir = plugins_dir.join(".cache");
    let cwasms: Vec<_> = std::fs::read_dir(&cache_dir)
        .expect(".cache should exist after the first discover")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "cwasm"))
        .collect();
    assert_eq!(cwasms.len(), 1, "exactly one .cwasm produced");
    let cwasm_path = cwasms[0].path();
    let mtime_first = std::fs::metadata(&cwasm_path).unwrap().modified().unwrap();

    {
        let loader = fresh_loader();
        let metas = loader.discover(&plugins_dir).await.unwrap();
        assert!(metas.iter().any(|m| m.id == "scripted"));
    }
    let mtime_second = std::fs::metadata(&cwasm_path).unwrap().modified().unwrap();
    assert_eq!(
        mtime_first, mtime_second,
        ".cwasm must be reused (unchanged mtime), not recompiled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_host_caller_reads_services() {
    let (_tmp, plugins_dir) = stage("host-caller");
    let loader = fresh_loader();
    let values = HashMap::from([("greeting".to_string(), "hello-wasm".to_string())]);
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            player_registry: Arc::new(CountingPlayerRegistry { count: 7 }),
            config_service: Arc::new(MapConfigService { values }),
            ..EnvOptions::default()
        },
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "host-caller").await;

    let dir = plugins_dir.join("host-caller");
    assert_eq!(
        std::fs::read_to_string(dir.join("count.txt")).expect("count.txt"),
        "7",
        "guest read online_count via the host"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("greeting.txt")).expect("greeting.txt"),
        "hello-wasm",
        "guest read a config value via the host"
    );
}

struct CommandPluginEnv {
    _tmp: tempfile::TempDir,
    plugins_dir: PathBuf,
    env: TestEnv,
    _loader: WasmPluginLoader,
    _plugin: Box<dyn Plugin>,
}

async fn enable_command_plugin() -> CommandPluginEnv {
    let (tmp, plugins_dir) = stage("command-plugin");
    let loader = fresh_loader();
    let env = make_env(plugins_dir.clone());
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = load_enabled(&loader, &env.factory, "command-plugin").await;
    CommandPluginEnv {
        _tmp: tmp,
        plugins_dir,
        env,
        _loader: loader,
        _plugin: plugin,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_command_plugin_dispatch_reaches_guest() {
    let fx = enable_command_plugin().await;

    let found = fx
        .env
        .command_manager
        .dispatch(None, "greet world peace", &MockPlayerRegistry)
        .await;
    assert!(
        found,
        "the guest-registered 'greet' command should be found"
    );

    let marker = fx.plugins_dir.join("command-plugin").join("command.marker");
    let got =
        std::fs::read_to_string(&marker).expect("guest should record the command args on dispatch");
    assert_eq!(got, "world,peace", "command args reached the guest");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_command_plugin_tab_complete_reaches_guest() {
    let fx = enable_command_plugin().await;

    let one = fx.env.command_manager.tab_complete("greet w").await;
    assert_eq!(
        one,
        vec!["world".to_string()],
        "prefix 'w' completes to exactly 'world' through the guest completer"
    );

    let all = fx.env.command_manager.tab_complete("greet ").await;
    assert_eq!(
        all.len(),
        3,
        "an empty prefix offers all three guest candidates"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_command_plugin_completer_can_register_a_command() {
    let fx = enable_command_plugin().await;

    assert_eq!(
        fx.env.command_manager.tab_complete("nest ").await,
        vec!["registered".to_string()],
        "a completer registering a command must return its candidates, not trap"
    );
    assert_eq!(
        fx.env.command_manager.tab_complete("nested ").await,
        vec!["inner".to_string()],
        "the command registered from the completer carries its own completer"
    );
    assert!(
        fx.env
            .command_manager
            .dispatch(None, "nested", &MockPlayerRegistry)
            .await,
        "the command registered from the completer is dispatchable"
    );
    assert_eq!(
        std::fs::read_to_string(fx.plugins_dir.join("command-plugin").join("nested.marker"))
            .expect("nested command ran in the guest"),
        "ran"
    );
    assert_eq!(
        fx.env.command_manager.tab_complete("greet w").await,
        vec!["world".to_string()],
        "the instance is still healthy afterwards"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_command_plugin_unregister_reaches_host() {
    let fx = enable_command_plugin().await;
    let marker = fx.plugins_dir.join("command-plugin").join("unnest.marker");

    fx.env.command_manager.tab_complete("nest ").await;
    assert!(
        fx.env
            .command_manager
            .dispatch(None, "unnest", &MockPlayerRegistry)
            .await
    );
    assert_eq!(
        std::fs::read_to_string(&marker).expect("unnest ran"),
        "true",
        "the guest owned `nested` and removed it"
    );
    assert!(
        !fx.env
            .command_manager
            .dispatch(None, "nested", &MockPlayerRegistry)
            .await,
        "the host no longer routes `nested`"
    );

    assert!(
        fx.env
            .command_manager
            .dispatch(None, "unnest", &MockPlayerRegistry)
            .await
    );
    assert_eq!(
        std::fs::read_to_string(&marker).expect("unnest ran again"),
        "false",
        "a second unregister finds nothing to remove"
    );
    assert!(
        fx.env
            .command_manager
            .dispatch(None, "greet again", &MockPlayerRegistry)
            .await,
        "other commands are untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stats_count_command() {
    let (_tmp, plugins_dir) = stage("stats");
    let loader = fresh_loader();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            player_registry: Arc::new(RecordingPlayerRegistry {
                count: 7,
                sent: Arc::clone(&sent),
            }),
            ..EnvOptions::default()
        },
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "stats").await;

    let found = env
        .command_manager
        .dispatch(Some(PlayerId::new(1)), "count", &MockPlayerRegistry)
        .await;
    assert!(found, "the stats 'count' command should be registered");

    assert_eq!(
        sent.lock().unwrap().as_slice(),
        ["Online: 7".to_string()],
        "wasm /count replied with the formatted online count"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_capability_denied_fails_to_load() {
    let (_tmp, plugins_dir) = stage("capability-denied");
    let loader = fresh_loader();
    let factory = make_factory(&plugins_dir);
    loader.discover(&plugins_dir).await.unwrap();

    let result = loader.load("capability-denied", &factory).await;
    assert!(
        result.is_err(),
        "a plugin importing an ungranted gated interface must fail to instantiate"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_per_plugin_memory_limit_applies_to_that_plugin_only() {
    let (_tmp, plugins_dir) = stage("scripted");
    add_fixture(&plugins_dir, "scripted-peer", "scripted-peer");
    write_script(&plugins_dir, "scripted", "");
    write_script(&plugins_dir, "scripted-peer", "");
    let loader = loader_from_toml("[plugins.scripted.wasm]\nmemory_limit_mb = 1\n");
    let factory = make_factory(&plugins_dir);
    loader.discover(&plugins_dir).await.unwrap();

    let refused = loader.load("scripted", &factory).await;
    let err = refused
        .err()
        .expect("a 1 MiB cap cannot hold the fixture's initial linear memory");
    let message = err.to_string();
    assert!(
        message.contains("scripted") && message.contains("growing memory"),
        "{message}"
    );

    let _peer = load_enabled(&loader, &factory, "scripted-peer").await;
    assert_eq!(
        read_log(&plugins_dir.join("scripted-peer")),
        ["enable"],
        "a plugin without an override keeps the 64 MiB default"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_denied_baseline_capability_is_refused_like_an_ungranted_one() {
    let (_tmp, plugins_dir) = stage("host-caller");
    let loader = fresh_loader();
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions::default().deny("host-caller", "config-read"),
    );
    loader.discover(&plugins_dir).await.unwrap();

    let err = loader
        .load("host-caller", &env.factory)
        .await
        .err()
        .expect("config-read is baseline, but denied it must not be linked");
    let message = err.to_string();
    assert!(message.contains("lacks the capability"), "{message}");
    assert!(
        message.contains("infrarust:plugin/config-service"),
        "{message}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_denied_player_write_stops_messages_to_players() {
    let (_tmp, plugins_dir) = stage("stats");
    let loader = fresh_loader();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            player_registry: Arc::new(RecordingPlayerRegistry {
                count: 7,
                sent: Arc::clone(&sent),
            }),
            ..EnvOptions::default()
        }
        .deny("stats", "player-write"),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "stats").await;

    assert!(
        env.command_manager
            .dispatch(Some(PlayerId::new(1)), "count", &MockPlayerRegistry)
            .await
    );
    assert!(
        sent.lock().unwrap().is_empty(),
        "player-write is denied, so the reply never reaches the player"
    );
}

const SLOW_HANDLER_TIMEOUT: Duration = Duration::from_millis(500);
const PATIENT_HANDLER_TIMEOUT: Duration = Duration::from_secs(30);
const PROMPTLY: Duration = Duration::from_secs(10);

async fn enable_slow_handler_with(
    loader: &WasmPluginLoader,
    plugins_dir: &Path,
    ban_service: Arc<dyn infrarust_api::services::ban_service::BanService>,
    handler_timeout: Duration,
) -> (TestEnv, Box<dyn Plugin>) {
    let env = make_env_with(
        plugins_dir.to_path_buf(),
        EnvOptions {
            ban_service,
            bus_config: EventBusConfig {
                handler_timeout,
                ..EventBusConfig::default()
            },
            ..EnvOptions::default()
        }
        .grant("slow-handler", "ban"),
    );
    loader.discover(plugins_dir).await.unwrap();
    let plugin = load_enabled(loader, &env.factory, "slow-handler").await;
    (env, plugin)
}

async fn enable_slow_handler(
    loader: &WasmPluginLoader,
    plugins_dir: &Path,
    gate: &Arc<Gate>,
) -> (TestEnv, Box<dyn Plugin>) {
    enable_slow_handler_with(
        loader,
        plugins_dir,
        Arc::new(GatedBanService {
            gate: Arc::clone(gate),
        }),
        SLOW_HANDLER_TIMEOUT,
    )
    .await
}

fn post_login() -> PostLoginEvent {
    PostLoginEvent::new(support::session_player(
        1,
        nil_profile("Steve"),
        ProtocolVersion::MINECRAFT_1_21.raw(),
        "127.0.0.1:40000".parse().unwrap(),
    ))
}

fn pre_connect() -> ServerPreConnectEvent {
    ServerPreConnectEvent::new(
        PlayerId::new(1),
        nil_profile("Steve"),
        ServerId::new("lobby"),
    )
}

fn outcome(event: &ServerPreConnectEvent) -> String {
    match event.result() {
        ServerPreConnectResult::Allowed => "allowed".to_string(),
        ServerPreConnectResult::ConnectTo(server) => format!("connect-to:{}", server.as_str()),
        _ => "other".to_string(),
    }
}

async fn poll_once<F: Future + Unpin>(future: &mut F) -> Option<F::Output> {
    std::future::poll_fn(|cx| {
        Poll::Ready(match Pin::new(&mut *future).poll(cx) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

async fn fire_post_login_past_timeout(env: &TestEnv) {
    let started = Instant::now();
    tokio::time::timeout(PROMPTLY, env.event_bus.fire(post_login()))
        .await
        .expect("the bus must give up on the handler at its timeout, not wait out the host call");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= SLOW_HANDLER_TIMEOUT && elapsed < PROMPTLY,
        "fire returned after {elapsed:?}, expected about {SLOW_HANDLER_TIMEOUT:?}"
    );
}

#[derive(Clone)]
struct LogCapture {
    level: tracing::Level,
    lines: Arc<Mutex<Vec<String>>>,
}

impl LogCapture {
    fn at(level: tracing::Level) -> Self {
        Self {
            level,
            lines: Arc::default(),
        }
    }

    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    fn matching(&self, needle: &str) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(needle))
            .collect()
    }
}

struct EventText(String);

impl tracing::field::Visit for EventText {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }
}

impl tracing::Subscriber for LogCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= self.level
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = EventText(format!("{} ", event.metadata().level()));
        event.record(&mut text);
        self.lines.lock().unwrap().push(text.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn test_handler_timeout_lets_the_guest_call_finish_and_keeps_the_instance_healthy() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let data = plugins_dir.join("slow-handler");
    let loader = fresh_loader();
    let gate = Gate::new();
    let errors = LogCapture::at(tracing::Level::ERROR);

    async {
        let (env, _plugin) = enable_slow_handler(&loader, &plugins_dir, &gate).await;

        fire_post_login_past_timeout(&env).await;
        gate.entered().await;
        assert!(
            read_log(&data).is_empty(),
            "the guest is still parked in its host call after the bus gave up"
        );

        gate.open();
        assert!(
            env.command_manager
                .dispatch(None, "ping", &MockPlayerRegistry)
                .await
        );
        assert_eq!(
            read_log(&data),
            ["post-login answered", "command"],
            "the timed-out handler finished inside the plugin, then the next call ran"
        );

        let event = env.event_bus.fire(pre_connect()).await;
        assert_eq!(
            outcome(&event),
            "connect-to:backend-1",
            "the next event is handled normally"
        );
        assert_eq!(
            env.command_manager.tab_complete("ping ").await,
            ["pong".to_string()]
        );
    }
    .with_subscriber(errors.clone())
    .await;

    for needle in ["poison", "abandoned", "trapped"] {
        assert!(
            errors.matching(needle).is_empty(),
            "a caller timeout must not poison the instance: {:?}",
            errors.lines()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_on_disable_runs_the_guest_after_a_timed_out_handler_returns() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let data = plugins_dir.join("slow-handler");
    let loader = fresh_loader();
    let gate = Gate::new();
    let (env, plugin) = enable_slow_handler(&loader, &plugins_dir, &gate).await;

    fire_post_login_past_timeout(&env).await;
    gate.entered().await;

    let mut disabling = plugin.on_disable();
    assert!(
        poll_once(&mut disabling).await.is_none(),
        "on_disable waits behind the handler still parked in its host call"
    );
    gate.open();
    let disabled = tokio::time::timeout(PROMPTLY, disabling)
        .await
        .expect("on_disable completes once the parked host call returns");

    assert!(disabled.is_ok(), "{disabled:?}");
    assert_eq!(
        read_log(&data),
        ["post-login answered", "disable"],
        "on_disable waited for the timed-out handler, then ran the guest in the same instance"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_full_queue_fails_fast_without_waiting_for_the_plugin() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let data = plugins_dir.join("slow-handler");
    let loader = loader_from_toml("[plugins.slow-handler.wasm]\nqueue_capacity = 1\n");
    let gate = Gate::new();
    let warnings = LogCapture::at(tracing::Level::WARN);

    async {
        let (env, _plugin) = enable_slow_handler_with(
            &loader,
            &plugins_dir,
            Arc::new(GatedBanService {
                gate: Arc::clone(&gate),
            }),
            PATIENT_HANDLER_TIMEOUT,
        )
        .await;

        let mut parked = Box::pin(env.event_bus.fire(post_login()));
        assert!(poll_once(&mut parked).await.is_none());
        gate.entered().await;
        let mut queued = Box::pin(env.event_bus.fire(pre_connect()));
        assert!(
            poll_once(&mut queued).await.is_none(),
            "the second call waits in the queue behind the parked one"
        );

        for _ in 0..3 {
            let rejected = tokio::time::timeout(PROMPTLY, env.event_bus.fire(pre_connect()))
                .await
                .expect("a full queue must not make the caller wait for the parked guest");
            assert_eq!(
                outcome(&rejected),
                "allowed",
                "a rejected call has no outcome"
            );
        }
        let found = tokio::time::timeout(
            PROMPTLY,
            env.command_manager
                .dispatch(None, "ping", &MockPlayerRegistry),
        )
        .await
        .expect("a command to a saturated plugin returns without waiting");
        assert!(found);

        gate.open();
        let queued = tokio::time::timeout(PROMPTLY, queued).await.unwrap();
        tokio::time::timeout(PROMPTLY, parked).await.unwrap();
        assert_eq!(
            outcome(&queued),
            "connect-to:backend-1",
            "the call that fit in the queue still ran"
        );
        assert_eq!(
            read_log(&data),
            ["post-login answered", "pre-connect"],
            "the rejected calls never reached the guest"
        );
    }
    .with_subscriber(warnings.clone())
    .await;

    let full = warnings.matching("queue is full");
    assert_eq!(full.len(), 1, "the warning is rate-limited: {full:?}");
    assert!(
        full[0].contains("slow-handler") && full[0].contains("handle-event"),
        "the warning names the plugin and the operation: {full:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_whose_caller_gave_up_while_queued_is_skipped() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let data = plugins_dir.join("slow-handler");
    let loader = fresh_loader();
    let gate = Gate::new();
    let debug = LogCapture::at(tracing::Level::DEBUG);

    async {
        let (env, _plugin) = enable_slow_handler(&loader, &plugins_dir, &gate).await;

        fire_post_login_past_timeout(&env).await;
        gate.entered().await;
        let abandoned = env.event_bus.fire(pre_connect()).await;
        assert_eq!(
            outcome(&abandoned),
            "allowed",
            "the bus gave up on the queued call"
        );

        gate.open();
        assert!(
            env.command_manager
                .dispatch(None, "ping", &MockPlayerRegistry)
                .await
        );
        assert_eq!(
            read_log(&data),
            ["post-login answered", "command"],
            "the guest never saw the event whose caller gave up while it was queued"
        );
    }
    .with_subscriber(debug.clone())
    .await;

    let skipped = debug.matching("stopped waiting");
    assert_eq!(skipped.len(), 1, "{:?}", debug.lines());
    assert!(skipped[0].contains("handle-event"), "{skipped:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_unload_stops_the_plugin_task() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let data = plugins_dir.join("slow-handler");
    let loader = fresh_loader();
    let gate = Gate::new();
    let (env, plugin) = enable_slow_handler(&loader, &plugins_dir, &gate).await;
    let context = Arc::downgrade(&env.factory.create_context("slow-handler"));

    loader.unload("slow-handler").await.expect("unload ok");
    assert!(
        context.upgrade().is_none(),
        "the plugin task ended and dropped its store along with the plugin context"
    );

    let found = tokio::time::timeout(
        PROMPTLY,
        env.command_manager
            .dispatch(None, "ping", &MockPlayerRegistry),
    )
    .await
    .expect("a call into an unloaded plugin returns promptly");
    assert!(found, "the host still routes the command");
    let event = tokio::time::timeout(PROMPTLY, env.event_bus.fire(pre_connect()))
        .await
        .expect("an event for an unloaded plugin returns promptly");
    assert_eq!(outcome(&event), "allowed");
    assert!(
        tokio::time::timeout(PROMPTLY, plugin.on_disable())
            .await
            .expect("on_disable of an unloaded plugin returns promptly")
            .is_ok()
    );
    assert!(read_log(&data).is_empty(), "no guest code ran after unload");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_past_max_call_duration_poisons_the_instance() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let data = plugins_dir.join("slow-handler");
    let loader = loader_from_toml("[plugins.slow-handler.wasm]\nmax_call_duration = \"200ms\"\n");
    let gate = Gate::new();
    let errors = LogCapture::at(tracing::Level::ERROR);

    async {
        let (env, plugin) = enable_slow_handler(&loader, &plugins_dir, &gate).await;

        tokio::time::timeout(PROMPTLY, env.event_bus.fire(post_login()))
            .await
            .expect("max_call_duration cuts off a guest call parked in a host call");
        let event = env.event_bus.fire(pre_connect()).await;
        assert_eq!(
            outcome(&event),
            "allowed",
            "an instance whose call was cut off gives no outcome"
        );
        assert!(
            env.command_manager
                .dispatch(None, "ping", &MockPlayerRegistry)
                .await
        );
        let disabled = tokio::time::timeout(PROMPTLY, plugin.on_disable())
            .await
            .expect("on_disable must not hang on a poisoned instance");
        assert!(disabled.is_ok(), "{disabled:?}");
        assert!(
            read_log(&data).is_empty(),
            "no guest code ran after the call was cut off"
        );
    }
    .with_subscriber(errors.clone())
    .await;

    assert_eq!(
        errors.matching("max_call_duration").len(),
        1,
        "{:?}",
        errors.lines()
    );
    assert_eq!(
        errors.matching("abandoned mid-execution").len(),
        1,
        "the unfinished call poisons the instance once: {:?}",
        errors.lines()
    );
    assert!(
        errors.matching("trapped").is_empty(),
        "later calls are refused up front, not attempted: {:?}",
        errors.lines()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_host_panic_inside_a_guest_call_poisons_the_instance() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let data = plugins_dir.join("slow-handler");
    let loader = fresh_loader();
    let errors = LogCapture::at(tracing::Level::ERROR);

    async {
        let (env, plugin) = enable_slow_handler_with(
            &loader,
            &plugins_dir,
            Arc::new(PanickingBanService),
            PATIENT_HANDLER_TIMEOUT,
        )
        .await;

        tokio::time::timeout(PROMPTLY, env.event_bus.fire(post_login()))
            .await
            .expect("a panicking host call must not hang the caller");
        let event = env.event_bus.fire(pre_connect()).await;
        assert_eq!(outcome(&event), "allowed");
        let disabled = tokio::time::timeout(PROMPTLY, plugin.on_disable())
            .await
            .expect("on_disable must not hang");
        assert!(disabled.is_ok(), "{disabled:?}");
        assert!(read_log(&data).is_empty());
    }
    .with_subscriber(errors.clone())
    .await;

    assert_eq!(
        errors.matching("panicked in a host function").len(),
        1,
        "{:?}",
        errors.lines()
    );
    assert_eq!(
        errors.matching("abandoned mid-execution").len(),
        1,
        "{:?}",
        errors.lines()
    );
}
