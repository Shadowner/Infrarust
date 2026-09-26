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

use infrarust_api::command::{CommandContext, CommandHandler, CommandSource, CommandSpec};
use infrarust_api::event::BoxFuture;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{
    ConnectCause, ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::loader::{LoaderError, PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{PlayerId, ProtocolVersion, ServerId};
use infrarust_core::event_bus::EventBusConfig;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::services::command_manager::{CommandManagerImpl, DispatchOutcome};
use infrarust_loader_wasm::WasmPluginLoader;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::mock_services::{
    CountingPlayerRegistry, Gate, GatedBanService, MapConfigService, RecordingPlayerRegistry,
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
async fn a_plugin_built_for_the_old_contract_is_refused_with_a_rebuild_hint() {
    let (_tmp, plugins_dir) = stage("old-world");
    let err = fresh_loader()
        .discover(&plugins_dir)
        .await
        .expect_err("infrarust:plugin@0.2.3 components are not loaded");
    assert!(matches!(err, LoaderError::InvalidFormat { .. }), "{err:?}");
    let message = err.to_string();
    for needle in [
        "infrarust:plugin@0.2.3",
        "infrarust:plugin@0.3.x",
        "rebuild",
        "infrarust-plugin-sdk",
    ] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from {message}"
        );
    }
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

    let found = dispatch_line(&fx.env.command_manager, "greet world peace").await;
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

    let one = complete_line(&fx.env.command_manager, "greet w").await;
    assert_eq!(
        one,
        vec!["world".to_string()],
        "prefix 'w' completes to exactly 'world' through the guest completer"
    );

    let all = complete_line(&fx.env.command_manager, "greet ").await;
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
        complete_line(&fx.env.command_manager, "nest ").await,
        vec!["registered".to_string()],
        "a completer registering a command must return its candidates, not trap"
    );
    assert_eq!(
        complete_line(&fx.env.command_manager, "nested ").await,
        vec!["inner".to_string()],
        "the command registered from the completer carries its own completer"
    );
    assert!(
        dispatch_line(&fx.env.command_manager, "nested").await,
        "the command registered from the completer is dispatchable"
    );
    assert_eq!(
        std::fs::read_to_string(fx.plugins_dir.join("command-plugin").join("nested.marker"))
            .expect("nested command ran in the guest"),
        "ran"
    );
    assert_eq!(
        complete_line(&fx.env.command_manager, "greet w").await,
        vec!["world".to_string()],
        "the instance is still healthy afterwards"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_command_plugin_unregister_reaches_host() {
    let fx = enable_command_plugin().await;
    let marker = fx.plugins_dir.join("command-plugin").join("unnest.marker");

    complete_line(&fx.env.command_manager, "nest ").await;
    assert!(dispatch_line(&fx.env.command_manager, "unnest").await);
    assert_eq!(
        std::fs::read_to_string(&marker).expect("unnest ran"),
        "true",
        "the guest owned `nested` and removed it"
    );
    assert!(
        !dispatch_line(&fx.env.command_manager, "nested").await,
        "the host no longer routes `nested`"
    );

    assert!(dispatch_line(&fx.env.command_manager, "unnest").await);
    assert_eq!(
        std::fs::read_to_string(&marker).expect("unnest ran again"),
        "false",
        "a second unregister finds nothing to remove"
    );
    assert!(
        dispatch_line(&fx.env.command_manager, "greet again").await,
        "other commands are untouched"
    );
}

struct NativeNested {
    ran: Arc<Mutex<u32>>,
}

impl CommandHandler for NativeNested {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        *self.ran.lock().unwrap() += 1;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_guest_cannot_unregister_a_command_it_does_not_own() {
    let fx = enable_command_plugin().await;
    let ran = Arc::new(Mutex::new(0));
    let native = fx.env.factory.create_context("native");
    native
        .command_manager()
        .register(
            CommandSpec::new("nested"),
            Box::new(NativeNested {
                ran: Arc::clone(&ran),
            }),
        )
        .unwrap();

    complete_line(&fx.env.command_manager, "nest ").await;
    assert!(dispatch_line(&fx.env.command_manager, "unnest").await);
    assert_eq!(
        std::fs::read_to_string(fx.plugins_dir.join("command-plugin").join("unnest.marker"))
            .expect("unnest ran"),
        "false",
        "the host refused `nested`, so the guest knows it owns nothing to remove"
    );

    assert!(
        dispatch_line(&fx.env.command_manager, "nested").await,
        "the native plugin's `nested` survived the guest's unregister"
    );
    assert!(dispatch_line(&fx.env.command_manager, "native:nested").await);
    assert_eq!(*ran.lock().unwrap(), 2);
    assert!(
        !fx.plugins_dir
            .join("command-plugin")
            .join("nested.marker")
            .exists(),
        "the guest's refused `nested` never ran"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stats_count_command() {
    let (_tmp, plugins_dir) = stage("stats");
    let loader = fresh_loader();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(RecordingPlayerRegistry {
        count: 7,
        sent: Arc::clone(&sent),
    });
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            player_registry: Arc::clone(&registry) as _,
            ..EnvOptions::default()
        },
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "stats").await;

    let outcome = env
        .command_manager
        .dispatch(player_source(&registry), "count")
        .await;
    assert_eq!(
        outcome,
        DispatchOutcome::Executed,
        "the stats 'count' command should be registered"
    );

    assert_eq!(
        sent.lock().unwrap().as_slice(),
        ["Online: 7".to_string()],
        "wasm /count replied with the formatted online count"
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
async fn test_denied_player_write_stops_messages_to_players() {
    let (_tmp, plugins_dir) = stage("stats");
    let loader = fresh_loader();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(RecordingPlayerRegistry {
        count: 7,
        sent: Arc::clone(&sent),
    });
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            player_registry: Arc::clone(&registry) as _,
            ..EnvOptions::default()
        }
        .deny("stats", "player-write"),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "stats").await;

    assert_eq!(
        env.command_manager
            .dispatch(player_source(&registry), "count")
            .await,
        DispatchOutcome::Executed
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
        support::session_player(
            1,
            nil_profile("Steve"),
            ProtocolVersion::MINECRAFT_1_21.raw(),
            "127.0.0.1:40000".parse().unwrap(),
        ),
        ServerId::new("lobby"),
        None,
        ConnectCause::Initial,
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
        assert!(dispatch_line(&env.command_manager, "ping").await);
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
            complete_line(&env.command_manager, "ping ").await,
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
        let found = tokio::time::timeout(PROMPTLY, dispatch_line(&env.command_manager, "ping"))
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
        assert!(dispatch_line(&env.command_manager, "ping").await);
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

    let found = tokio::time::timeout(PROMPTLY, dispatch_line(&env.command_manager, "ping"))
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

async fn dispatch_line(commands: &CommandManagerImpl, line: &str) -> bool {
    commands.dispatch(support::console(), line).await == DispatchOutcome::Executed
}

async fn complete_line(commands: &CommandManagerImpl, input: &str) -> Vec<String> {
    commands
        .suggest(support::console(), input)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|suggestion| suggestion.text)
        .collect()
}

fn player_source(registry: &RecordingPlayerRegistry) -> CommandSource {
    CommandSource::Player(
        registry
            .get_player_by_id(PlayerId::new(1))
            .expect("the recording registry knows every id"),
    )
}
