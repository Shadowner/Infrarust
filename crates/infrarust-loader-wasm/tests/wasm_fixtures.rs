#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    CountingPlayerRegistry, MapConfigService, MockPlayerRegistry, PendingBanService,
    RecordingPlayerRegistry,
};
use support::{
    EnvOptions, TestEnv, add_fixture, fresh_loader, load_enabled, make_env, make_env_with,
    nil_profile, read_log, stage, write_script,
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

const SLOW_HANDLER_TIMEOUT: Duration = Duration::from_millis(100);

async fn enable_slow_handler(
    loader: &WasmPluginLoader,
    plugins_dir: &Path,
) -> (TestEnv, Box<dyn Plugin>) {
    let env = make_env_with(
        plugins_dir.to_path_buf(),
        EnvOptions {
            ban_service: Arc::new(PendingBanService),
            bus_config: EventBusConfig {
                handler_timeout: SLOW_HANDLER_TIMEOUT,
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

async fn fire_post_login_past_timeout(env: &TestEnv, plugins_dir: &Path) {
    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(10),
        env.event_bus.fire(PostLoginEvent {
            profile: nil_profile("Steve"),
            player_id: PlayerId::new(1),
            protocol_version: ProtocolVersion::MINECRAFT_1_21,
        }),
    )
    .await
    .expect("the bus must cancel the handler at its timeout, not wait out the host call");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= SLOW_HANDLER_TIMEOUT && elapsed < Duration::from_secs(5),
        "fire returned after {elapsed:?}, expected about {SLOW_HANDLER_TIMEOUT:?}"
    );
    assert!(
        !plugins_dir
            .join("slow-handler")
            .join("post-login.marker")
            .exists(),
        "the handler was cancelled inside its host call and never finished"
    );
}

#[derive(Clone, Default)]
struct ErrorLog(Arc<Mutex<Vec<String>>>);

impl ErrorLog {
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct EventText(String);

impl tracing::field::Visit for EventText {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }
}

impl tracing::Subscriber for ErrorLog {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::ERROR
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = EventText(String::new());
        event.record(&mut text);
        self.0.lock().unwrap().push(text.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn test_handler_cancelled_mid_call_poisons_instance() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let loader = fresh_loader();
    let (env, _plugin) = enable_slow_handler(&loader, &plugins_dir).await;
    let data = plugins_dir.join("slow-handler");

    fire_post_login_past_timeout(&env, &plugins_dir).await;

    let errors = ErrorLog::default();
    let (event, found, completions) = async {
        let event = env
            .event_bus
            .fire(ServerPreConnectEvent::new(
                PlayerId::new(1),
                nil_profile("Steve"),
                ServerId::new("lobby"),
            ))
            .await;
        let found = env
            .command_manager
            .dispatch(None, "ping", &MockPlayerRegistry)
            .await;
        let completions = env.command_manager.tab_complete("ping ").await;
        (event, found, completions)
    }
    .with_subscriber(errors.clone())
    .await;

    assert!(
        matches!(event.result(), ServerPreConnectResult::Allowed),
        "the next event must get no outcome from the abandoned instance"
    );
    assert!(
        !data.join("pre-connect.marker").exists(),
        "the next event handler must not run in the abandoned store"
    );
    assert!(found, "the host still routes the guest command");
    assert!(
        !data.join("command.marker").exists(),
        "the command must not run in the abandoned store"
    );
    assert!(
        completions.is_empty(),
        "tab-complete must not run in the abandoned store"
    );
    let lines = errors.lines();
    assert_eq!(
        lines.iter().filter(|l| l.contains("abandoned")).count(),
        1,
        "the abandoned call is reported exactly once: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("trapped")),
        "later calls are refused up front, not attempted and reported as traps: {lines:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_on_disable_skips_instance_cancelled_mid_call() {
    let (_tmp, plugins_dir) = stage("slow-handler");
    let loader = fresh_loader();
    let (env, plugin) = enable_slow_handler(&loader, &plugins_dir).await;

    fire_post_login_past_timeout(&env, &plugins_dir).await;

    let disabled = tokio::time::timeout(Duration::from_secs(10), plugin.on_disable())
        .await
        .expect("on_disable must not hang on an abandoned instance");
    assert!(
        disabled.is_ok(),
        "on_disable skips the guest call on an abandoned instance: {disabled:?}"
    );
    assert!(
        !plugins_dir
            .join("slow-handler")
            .join("disable.marker")
            .exists(),
        "the guest on_disable must not run in the abandoned store"
    );
}
