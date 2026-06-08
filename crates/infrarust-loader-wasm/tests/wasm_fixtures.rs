//! The whole file compiles to nothing unless the `wasm` feature is on AND `build.rs`
//! managed to build the fixtures (the `wasm_fixtures_available` cfg), so a machine without
//! the `wasm32-wasip2` target degrades to "0 tests" rather than a spurious failure.

#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::command::CommandManager;
use infrarust_api::event::ResultedEvent;
use infrarust_api::event::bus::EventBus;
use infrarust_api::events::connection::{ServerPreConnectEvent, ServerPreConnectResult};
use infrarust_api::events::lifecycle::PostLoginEvent;
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion, ServerId};
use infrarust_config::ProxyConfig;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use infrarust_loader_wasm::{WasmPluginLoader, build_engine};

mod mock_services;
use mock_services::{
    CountingPlayerRegistry, MapConfigService, MockBanService, MockConfigService,
    MockPlayerRegistry, RecordingPlayerRegistry,
};

const FIXTURE_DIR: &str = env!("INFRARUST_WASM_FIXTURE_DIR");

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURE_DIR).join(format!("fixture_{}.wasm", name.replace('-', "_")))
}

fn fresh_loader() -> WasmPluginLoader {
    let config: ProxyConfig = toml::from_str("").expect("default proxy config");
    WasmPluginLoader::new(build_engine(&config).expect("build engine"))
}

struct TestEnv {
    factory: PluginContextFactoryImpl,
    event_bus: Arc<EventBusImpl>,
    command_manager: Arc<CommandManagerImpl>,
}

fn make_env(
    plugins_dir: PathBuf,
    player_registry: Arc<dyn PlayerRegistry>,
    config_service: Arc<dyn ConfigService>,
) -> TestEnv {
    let event_bus = Arc::new(EventBusImpl::new());
    let command_manager = Arc::new(CommandManagerImpl::new());
    let services = PluginServices {
        event_bus: Arc::clone(&event_bus) as Arc<dyn EventBus>,
        player_registry,
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::clone(&command_manager) as Arc<dyn CommandManager>,
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service,
        plugin_registry: Arc::new(infrarust_core::plugin::PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        domain_router: Arc::new(infrarust_core::routing::DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        proxy_info: infrarust_api::services::proxy_info::ProxyInfo::default(),
        plugins_dir,
    };
    TestEnv {
        factory: PluginContextFactoryImpl::new(services, HashMap::new()),
        event_bus,
        command_manager,
    }
}

fn make_factory(plugins_dir: PathBuf) -> PluginContextFactoryImpl {
    make_env(
        plugins_dir,
        Arc::new(MockPlayerRegistry),
        Arc::new(MockConfigService),
    )
    .factory
}

async fn load_enabled(
    loader: &WasmPluginLoader,
    factory: &PluginContextFactoryImpl,
    id: &str,
) -> Box<dyn Plugin> {
    let plugin = loader
        .load(id, factory)
        .await
        .unwrap_or_else(|e| panic!("load {id}: {e}"));
    let ctx = factory.create_context(id);
    plugin
        .on_enable(ctx.as_ref())
        .await
        .unwrap_or_else(|e| panic!("enable {id}: {e}"));
    plugin
}

fn nil_profile(username: &str) -> GameProfile {
    GameProfile {
        uuid: uuid::Uuid::nil(),
        username: username.to_string(),
        properties: vec![],
    }
}

/// Stages a single fixture under a fresh temp plugin dir.
fn stage(fixture: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(
        fixture_path(fixture),
        plugins_dir.join(format!("{fixture}.wasm")),
    )
    .unwrap();
    (tmp, plugins_dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_hello_loads_enables_disables() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(fixture_path("hello"), plugins_dir.join("hello.wasm")).unwrap();

    let loader = fresh_loader();
    let factory = make_factory(plugins_dir.clone());

    let metas = loader.discover(&plugins_dir).await.unwrap();
    assert!(metas.iter().any(|m| m.id == "hello"), "hello discovered");

    let plugin = loader.load("hello", &factory).await.expect("load hello");
    let ctx = factory.create_context("hello");
    plugin.on_enable(ctx.as_ref()).await.expect("on_enable ok");

    let marker = plugins_dir.join("hello").join("enabled.marker");
    assert!(marker.exists(), "on_enable should write enabled.marker");

    plugin.on_disable().await.expect("on_disable ok");
    loader.unload("hello").await.expect("unload ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sdk_smoke_loads_via_library_split() {
    let (_tmp, plugins_dir) = stage("sdk-smoke");
    let loader = fresh_loader();
    let factory = make_factory(plugins_dir.clone());

    let metas = loader.discover(&plugins_dir).await.unwrap();
    assert!(
        metas.iter().any(|m| m.id == "sdk-smoke"),
        "sdk-smoke metadata read through the SDK library split"
    );

    let _plugin = load_enabled(&loader, &factory, "sdk-smoke").await;
    let marker = plugins_dir.join("sdk-smoke").join("sdk-smoke.marker");
    assert!(
        marker.exists(),
        "on_enable ran through the SDK-exported component"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trap_on_purpose_is_contained() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(
        fixture_path("trap-on-purpose"),
        plugins_dir.join("trap.wasm"),
    )
    .unwrap();
    std::fs::copy(fixture_path("hello"), plugins_dir.join("hello.wasm")).unwrap();

    let loader = fresh_loader();
    let factory = make_factory(plugins_dir.clone());
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

    let hello = loader
        .load("hello", &factory)
        .await
        .expect("engine still usable after a trap");
    let hello_ctx = factory.create_context("hello");
    hello
        .on_enable(hello_ctx.as_ref())
        .await
        .expect("hello enables after a trap");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cpu_spin_interrupted_by_epoch() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(fixture_path("cpu-spin"), plugins_dir.join("cpu.wasm")).unwrap();

    let loader = fresh_loader();
    let factory = make_factory(plugins_dir.clone());
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
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(fixture_path("memory-bomb"), plugins_dir.join("bomb.wasm")).unwrap();

    let loader = fresh_loader();
    let factory = make_factory(plugins_dir.clone());
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
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(fixture_path("hello"), plugins_dir.join("hello.wasm")).unwrap();

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
        assert!(metas.iter().any(|m| m.id == "hello"));
    }
    let mtime_second = std::fs::metadata(&cwasm_path).unwrap().modified().unwrap();
    assert_eq!(
        mtime_first, mtime_second,
        ".cwasm must be reused (unchanged mtime), not recompiled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_event_subscriber_receives_post_login() {
    let (_tmp, plugins_dir) = stage("event-subscriber");
    let loader = fresh_loader();
    let env = make_env(
        plugins_dir.clone(),
        Arc::new(MockPlayerRegistry),
        Arc::new(MockConfigService),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "event-subscriber").await;

    // Fire a PostLoginEvent on the same bus the guest subscribed to.
    env.event_bus
        .fire(PostLoginEvent {
            profile: nil_profile("Steve"),
            player_id: PlayerId::new(1),
            protocol_version: ProtocolVersion::MINECRAFT_1_21,
        })
        .await;

    let marker = plugins_dir
        .join("event-subscriber")
        .join("post-login.marker");
    let got = std::fs::read_to_string(&marker)
        .expect("guest should have written post-login.marker on receipt");
    assert_eq!(got, "Steve", "guest saw the joining player's username");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_handler_ordering_and_independent_cancel() {
    let (_tmp, plugins_dir) = stage("multi-handler");
    let loader = fresh_loader();
    let env = make_env(
        plugins_dir.clone(),
        Arc::new(MockPlayerRegistry),
        Arc::new(MockConfigService),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "multi-handler").await;

    let marker = plugins_dir.join("multi-handler").join("multi.marker");

    env.event_bus
        .fire(PostLoginEvent {
            profile: nil_profile("Steve"),
            player_id: PlayerId::new(1),
            protocol_version: ProtocolVersion::MINECRAFT_1_21,
        })
        .await;
    assert_eq!(
        std::fs::read_to_string(&marker).expect("marker written on first fire"),
        "ABCD",
        "handlers ran in priority order (custom interleaved) with the cancelled one absent"
    );

    env.event_bus
        .fire(PostLoginEvent {
            profile: nil_profile("Alex"),
            player_id: PlayerId::new(2),
            protocol_version: ProtocolVersion::MINECRAFT_1_21,
        })
        .await;
    assert_eq!(
        std::fs::read_to_string(&marker).expect("marker after second fire"),
        "ABCDABCD",
        "each handler fired exactly once per event (no double-fire)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_event_modifier_applies_outcome() {
    let (_tmp, plugins_dir) = stage("event-modifier");
    let loader = fresh_loader();
    let env = make_env(
        plugins_dir.clone(),
        Arc::new(MockPlayerRegistry),
        Arc::new(MockConfigService),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "event-modifier").await;

    let event = ServerPreConnectEvent::new(
        PlayerId::new(1),
        nil_profile("Steve"),
        ServerId::new("lobby"),
    );
    let event = env.event_bus.fire(event).await;

    match event.result() {
        ServerPreConnectResult::ConnectTo(target) => {
            assert_eq!(
                target.as_str(),
                "backend-1",
                "guest redirected the connection"
            );
        }
        _ => panic!("expected ServerPreConnectResult::ConnectTo(backend-1)"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_host_caller_reads_services() {
    let (_tmp, plugins_dir) = stage("host-caller");
    let loader = fresh_loader();
    let mut values = HashMap::new();
    values.insert("greeting".to_string(), "hello-wasm".to_string());
    let env = make_env(
        plugins_dir.clone(),
        Arc::new(CountingPlayerRegistry { count: 7 }),
        Arc::new(MapConfigService { values }),
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

#[tokio::test(flavor = "multi_thread")]
async fn test_command_plugin_dispatch_reaches_guest() {
    let (_tmp, plugins_dir) = stage("command-plugin");
    let loader = fresh_loader();
    let env = make_env(
        plugins_dir.clone(),
        Arc::new(MockPlayerRegistry),
        Arc::new(MockConfigService),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "command-plugin").await;

    let found = env
        .command_manager
        .dispatch(None, "greet world peace", &MockPlayerRegistry)
        .await;
    assert!(
        found,
        "the guest-registered 'greet' command should be found"
    );

    let marker = plugins_dir.join("command-plugin").join("command.marker");
    let got =
        std::fs::read_to_string(&marker).expect("guest should record the command args on dispatch");
    assert_eq!(got, "world,peace", "command args reached the guest");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_command_plugin_tab_complete_reaches_guest() {
    let (_tmp, plugins_dir) = stage("command-plugin");
    let loader = fresh_loader();
    let env = make_env(
        plugins_dir.clone(),
        Arc::new(MockPlayerRegistry),
        Arc::new(MockConfigService),
    );
    loader.discover(&plugins_dir).await.unwrap();
    let _plugin = load_enabled(&loader, &env.factory, "command-plugin").await;

    let one = env.command_manager.tab_complete("greet w").await;
    assert_eq!(
        one,
        vec!["world".to_string()],
        "prefix 'w' completes to exactly 'world' through the guest completer"
    );

    let all = env.command_manager.tab_complete("greet ").await;
    assert_eq!(
        all.len(),
        3,
        "an empty prefix offers all three guest candidates"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_stats_count_command() {
    let (_tmp, plugins_dir) = stage("stats");
    let loader = fresh_loader();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let env = make_env(
        plugins_dir.clone(),
        Arc::new(RecordingPlayerRegistry {
            count: 7,
            sent: Arc::clone(&sent),
        }),
        Arc::new(MockConfigService),
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
    let factory = make_factory(plugins_dir.clone());
    loader.discover(&plugins_dir).await.unwrap();

    let result = loader.load("capability-denied", &factory).await;
    assert!(
        result.is_err(),
        "a plugin importing an ungranted gated interface must fail to instantiate"
    );
}
