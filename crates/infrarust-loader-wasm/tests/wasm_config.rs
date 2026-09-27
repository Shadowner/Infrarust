#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::Path;
use std::time::Duration;

use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;

use support::{fixture_path, loader_from_toml, make_env};

const MARKER: &[u8] = b"LIFPROBE-BLOB-V1";
const ENABLE_BOUND: Duration = Duration::from_secs(10);

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

async fn enable(
    loader: &infrarust_loader_wasm::WasmPluginLoader,
    factory: &infrarust_core::plugin::PluginContextFactoryImpl,
    id: &str,
) -> Result<Box<dyn Plugin>, String> {
    let plugin = loader.load(id, factory).await.map_err(|e| e.to_string())?;
    let ctx = factory.create_context(id);
    tokio::time::timeout(ENABLE_BOUND, plugin.on_enable(ctx.as_ref()))
        .await
        .map_err(|_| "on_enable hung".to_owned())?
        .map_err(|e| e.to_string())?;
    Ok(plugin)
}

#[tokio::test(flavor = "multi_thread")]
async fn instance_pool_exhaustion_fails_only_the_extra_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    for id in ["p1", "p2", "p3"] {
        add_probe(&dir, id, &format!("id={id}\n"));
    }
    let loader = loader_from_toml("[wasm]\ninstance_pool = 2\n");
    loader.discover(&dir).await.unwrap();
    let factory = make_env(dir.clone()).factory;

    let first = enable(&loader, &factory, "p1").await;
    let second = enable(&loader, &factory, "p2").await;
    let third = enable(&loader, &factory, "p3").await;

    let _held = (first.as_ref().ok(), second.as_ref().ok());
    eprintln!(
        "instance_pool = 2: p1 ok={}, p2 ok={}, p3 ok={}",
        first.is_ok(),
        second.is_ok(),
        third.is_ok()
    );
    assert!(
        first.is_ok() && second.is_ok(),
        "the first two must fit the pool: p1={:?} p2={:?}",
        first.as_ref().err(),
        second.as_ref().err()
    );
    if let Err(reason) = &third {
        assert!(
            !reason.contains("panic") && !reason.contains("not found"),
            "pool exhaustion must be a clear refusal, got: {reason}"
        );
    }
    assert!(
        third.is_err(),
        "a third plugin must not silently exceed instance_pool = 2 \
         (if it loads, the pool limit is not enforced)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_per_plugin_memory_override_binds_to_that_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "tight", "id=tight\n");
    add_probe(&dir, "roomy", "id=roomy\n");
    let loader = loader_from_toml("[plugins.tight.wasm]\nmemory_limit_mb = 1\n");
    loader.discover(&dir).await.unwrap();
    let factory = make_env(dir.clone()).factory;

    let tight = enable(&loader, &factory, "tight").await;
    assert!(
        tight.is_err(),
        "a 1 MiB cap must refuse the fixture's initial linear memory"
    );
    let roomy = enable(&loader, &factory, "roomy").await;
    assert!(
        roomy.is_ok(),
        "the default 64 MiB cap must hold the same fixture: {:?}",
        roomy.as_ref().err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_per_plugin_cpu_budget_override_binds_to_that_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "slowen", "id=slowen\nenable=spin\n");
    let loader = loader_from_toml("[plugins.slowen.wasm]\ncpu_budget = \"300ms\"\n");
    loader.discover(&dir).await.unwrap();
    let factory = make_env(dir.clone()).factory;

    let started = std::time::Instant::now();
    let outcome = enable(&loader, &factory, "slowen").await;
    let elapsed = started.elapsed();
    eprintln!("cpu_budget = 300ms on a spinning on_enable: {elapsed:?}, ok={}", outcome.is_ok());
    assert!(outcome.is_err(), "a spinning on_enable must be cut, not enabled");
    assert!(
        elapsed < Duration::from_secs(3),
        "the 300ms per-plugin cpu_budget override was not applied: on_enable ran {elapsed:?}"
    );
}
