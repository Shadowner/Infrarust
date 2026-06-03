//! The whole file compiles to nothing unless the `wasm` feature is on AND `build.rs`
//! managed to build the fixtures (the `wasm_fixtures_available` cfg), so a machine without
//! the `wasm32-wasip2` target degrades to "0 tests" rather than a spurious failure.

#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_config::ProxyConfig;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use infrarust_loader_wasm::{WasmPluginLoader, build_engine};

mod mock_services;
use mock_services::{MockBanService, MockConfigService, MockPlayerRegistry};

const FIXTURE_DIR: &str = env!("INFRARUST_WASM_FIXTURE_DIR");

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURE_DIR).join(format!("fixture_{}.wasm", name.replace('-', "_")))
}

fn fresh_loader() -> WasmPluginLoader {
    let config: ProxyConfig = toml::from_str("").expect("default proxy config");
    WasmPluginLoader::new(build_engine(&config).expect("build engine"))
}

fn make_factory(plugins_dir: PathBuf) -> PluginContextFactoryImpl {
    let event_bus = Arc::new(EventBusImpl::new());
    let services = PluginServices {
        event_bus: event_bus as Arc<dyn infrarust_api::event::bus::EventBus>,
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
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
    PluginContextFactoryImpl::new(services, std::collections::HashMap::new())
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
async fn test_trap_on_purpose_is_contained() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(fixture_path("trap-on-purpose"), plugins_dir.join("trap.wasm")).unwrap();
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

    let plugin = loader.load("cpu-spin", &factory).await.expect("load cpu-spin");
    let ctx = factory.create_context("cpu-spin");

    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        plugin.on_enable(ctx.as_ref()),
    )
    .await;
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

    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        plugin.on_enable(ctx.as_ref()),
    )
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
