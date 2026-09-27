#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use infrarust_api::loader::{LoaderError, PluginContextFactory, PluginLoader};
use infrarust_api::plugin::PluginMetadata;

use support::{add_fixture, fixture_path, fresh_loader, loader_from_toml, make_env};

const MARKER: &[u8] = b"LIFPROBE-BLOB-V1";
const BOUND: Duration = Duration::from_secs(60);

fn probe_bytes(settings: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture_path("lif-probe")).unwrap();
    let at = bytes
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .unwrap()
        + MARKER.len();
    bytes[at..at + settings.len()].copy_from_slice(settings.as_bytes());
    bytes[at + settings.len()] = 0xFF;
    bytes
}

fn add_probe(dir: &Path, stem: &str, settings: &str) {
    std::fs::write(dir.join(format!("{stem}.wasm")), probe_bytes(settings)).unwrap();
}

async fn discover_with(
    loader: &infrarust_loader_wasm::WasmPluginLoader,
    dir: &Path,
) -> Result<Vec<PluginMetadata>, LoaderError> {
    tokio::time::timeout(BOUND, loader.discover(dir))
        .await
        .expect("discovery finishes in bounded time")
}

async fn discover(dir: &Path) -> Result<Vec<PluginMetadata>, LoaderError> {
    discover_with(&fresh_loader(), dir).await
}

fn ids(metas: &[PluginMetadata]) -> Vec<String> {
    let mut ids: Vec<String> = metas.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    ids
}

fn cache_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir.join(".cache"))
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    entries
}

fn cwasm_entries(dir: &Path) -> Vec<PathBuf> {
    cache_entries(dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "cwasm"))
        .collect()
}

fn only_cwasm(dir: &Path) -> PathBuf {
    let entries = cwasm_entries(dir);
    assert_eq!(entries.len(), 1, "{entries:?}");
    entries[0].clone()
}

async fn staged_good_with_cache() -> (tempfile::TempDir, PathBuf, PathBuf, Vec<u8>) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "good", "id=good\n");
    assert_eq!(ids(&discover(&dir).await.unwrap()), ["good"]);
    let cwasm = only_cwasm(&dir);
    let original = std::fs::read(&cwasm).unwrap();
    (tmp, dir, cwasm, original)
}

async fn assert_rebuilt(dir: &Path, cwasm: &Path, original: &[u8], what: &str) {
    let metas = discover(dir).await.unwrap_or_else(|e| {
        panic!("a {what} cache entry must be rebuilt, not fail discovery: {e}")
    });
    assert_eq!(ids(&metas), ["good"], "{what}");
    assert_eq!(
        std::fs::read(cwasm).unwrap(),
        original,
        "the {what} entry was replaced by a fresh compilation"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_corrupted_cache_entry_is_rebuilt() {
    let (_tmp, dir, cwasm, original) = staged_good_with_cache().await;
    let junk: Vec<u8> = (0..original.len()).map(|i| (i * 31 % 251) as u8).collect();
    std::fs::write(&cwasm, junk).unwrap();
    assert_rebuilt(&dir, &cwasm, &original, "corrupted").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_cache_entry_is_rebuilt() {
    let (_tmp, dir, cwasm, original) = staged_good_with_cache().await;
    for keep in [original.len() / 2, 4096, 64, 0] {
        std::fs::write(&cwasm, &original[..keep]).unwrap();
        assert_rebuilt(&dir, &cwasm, &original, &format!("truncated-to-{keep}")).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cache_entry_written_by_another_wasmtime_version_is_rebuilt() {
    let (_tmp, dir, cwasm, original) = staged_good_with_cache().await;
    let section = elf_section(&original, ".wasmtime.engine").expect("an engine section");
    assert_eq!(&original[section.start..section.start + 4], b"\x00\x0249");
    let mut foreign = original.clone();
    foreign[section.start + 2..section.start + 4].copy_from_slice(b"45");
    std::fs::write(&cwasm, &foreign).unwrap();
    assert_rebuilt(&dir, &cwasm, &original, "wasmtime-45").await;
}

fn le_u16(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn le_u32(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

fn le_u64(bytes: &[u8], at: usize) -> usize {
    usize::try_from(u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())).unwrap()
}

fn elf_section(elf: &[u8], wanted: &str) -> Option<std::ops::Range<usize>> {
    let table = le_u64(elf, 0x28);
    let entry_size = le_u16(elf, 0x3A);
    let count = le_u16(elf, 0x3C);
    let names_index = le_u16(elf, 0x3E);
    let header = |index: usize| table + index * entry_size;
    let names = le_u64(elf, header(names_index) + 0x18);
    (0..count).find_map(|index| {
        let at = header(index);
        let name_at = names + le_u32(elf, at);
        let end = elf[name_at..].iter().position(|b| *b == 0)? + name_at;
        (&elf[name_at..end] == wanted.as_bytes()).then(|| {
            let offset = le_u64(elf, at + 0x18);
            offset..offset + le_u64(elf, at + 0x20)
        })
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cache_entry_compiled_without_epoch_interruption_is_rebuilt_and_the_spin_is_still_cut() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_fixture(&dir, "cpu-spin", "cpu-spin");
    discover(&dir).await.unwrap();
    let cwasm = only_cwasm(&dir);
    let original = std::fs::read(&cwasm).unwrap();

    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(false);
    let foreign_engine = wasmtime::Engine::new(&config).unwrap();
    let foreign = foreign_engine
        .precompile_component(&std::fs::read(fixture_path("cpu-spin")).unwrap())
        .unwrap();
    assert_ne!(foreign, original);
    std::fs::write(&cwasm, &foreign).unwrap();

    let loader = fresh_loader();
    discover_with(&loader, &dir).await.unwrap();
    assert_eq!(
        std::fs::read(&cwasm).unwrap(),
        original,
        "an artifact without epoch checks is replaced"
    );
    let env = make_env(dir.clone());
    let plugin = loader.load("cpu-spin", &env.factory).await.unwrap();
    let ctx = env.factory.create_context("cpu-spin");
    let outcome = tokio::time::timeout(Duration::from_secs(15), plugin.on_enable(ctx.as_ref()))
        .await
        .expect("the spin is interrupted, so the foreign artifact was not used");
    assert!(outcome.is_err());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-17: AOT cache entry runs without proof of origin"]
async fn a_cache_entry_is_bound_to_the_component_it_was_compiled_from() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "good", "id=good\n");
    discover(&dir).await.unwrap();
    let good_entry = only_cwasm(&dir);

    let other = tempfile::tempdir().unwrap();
    add_probe(other.path(), "other", "id=swapped-in\n");
    discover(other.path()).await.unwrap();
    let other_entry = only_cwasm(other.path());
    std::fs::copy(&other_entry, &good_entry).unwrap();

    let metas = discover(&dir).await.unwrap();
    assert_eq!(
        ids(&metas),
        ["good"],
        "good.wasm must run good.wasm's code, not whatever artifact sits under its cache key"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-11: unwritable AOT cache refuses every plugin"]
async fn a_read_only_cache_dir_does_not_stop_loading() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "good", "id=good\n");
    let cache = dir.join(".cache");
    std::fs::create_dir(&cache).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(cache.join("probe")).is_ok() {
        return;
    }
    let found = discover(&dir).await;
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755)).unwrap();
    let metas = found.unwrap_or_else(|e| {
        panic!("an unwritable cache must degrade to in-memory compilation, not fail: {e}")
    });
    assert_eq!(ids(&metas), ["good"]);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-11: unwritable AOT cache refuses every plugin"]
async fn a_read_only_plugins_dir_still_loads_its_plugins() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("plugins");
    std::fs::create_dir(&dir).unwrap();
    add_probe(&dir, "good", "id=good\n");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(dir.join("probe")).is_ok() {
        return;
    }
    let found = discover(&dir).await;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let metas = found.unwrap_or_else(|e| {
        panic!("a read-only plugins_dir (for example a mounted volume) must still load: {e}")
    });
    assert_eq!(ids(&metas), ["good"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-18: cache temp files named by PID"]
async fn concurrent_loaders_sharing_one_cache_dir_all_succeed() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "good", "id=good\n");
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let dir = dir.clone();
        tasks.push(tokio::spawn(async move {
            let loader = fresh_loader();
            discover_with(&loader, &dir).await.map(|metas| ids(&metas))
        }));
    }
    let mut failures = Vec::new();
    for task in tasks {
        match task.await.unwrap() {
            Ok(found) => assert_eq!(found, ["good"]),
            Err(e) => failures.push(e.to_string()),
        }
    }
    assert!(
        failures.is_empty(),
        "concurrent discoveries failed: {failures:#?}"
    );
    let leftovers: Vec<PathBuf> = cache_entries(&dir)
        .into_iter()
        .filter(|path| path.extension().is_none_or(|ext| ext != "cwasm"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left in .cache: {leftovers:?}"
    );
    assert_eq!(cwasm_entries(&dir).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "W-19: AOT cache grows without bound"]
async fn the_cache_does_not_grow_without_bound_across_plugin_updates() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let updates = 12;
    for release in 0..updates {
        add_probe(&dir, "good", &format!("id=good\nversion=0.1.{release}\n"));
        let metas = discover(&dir).await.unwrap();
        assert_eq!(metas[0].version, format!("0.1.{release}"));
    }
    let entries = cwasm_entries(&dir);
    let bytes: u64 = entries
        .iter()
        .map(|path| std::fs::metadata(path).unwrap().len())
        .sum();
    eprintln!(
        "after {updates} updates of one plugin: {} .cwasm entries, {bytes} bytes",
        entries.len()
    );
    assert!(
        entries.len() <= 2,
        "one plugin updated {updates} times leaves {} artifacts ({bytes} bytes) in .cache",
        entries.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn toggling_instance_pool_does_not_recompile_on_every_start() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "good", "id=good\n");
    let mut rebuilds = Vec::new();
    let mut previous: Option<Vec<u8>> = None;
    for pool in [0u32, 8, 0, 8] {
        let loader = loader_from_toml(&format!("[wasm]\ninstance_pool = {pool}\n"));
        let before = std::fs::metadata(dir.join(".cache")).ok().and_then(|_| {
            cwasm_entries(&dir)
                .first()
                .map(|p| std::fs::metadata(p).unwrap().modified().unwrap())
        });
        discover_with(&loader, &dir).await.unwrap();
        let entry = only_cwasm(&dir);
        let after = std::fs::metadata(&entry).unwrap().modified().unwrap();
        let bytes = std::fs::read(&entry).unwrap();
        let changed = before.is_some_and(|before| before != after)
            || previous.as_ref().is_some_and(|previous| *previous != bytes);
        rebuilds.push((pool, changed));
        previous = Some(bytes);
    }
    eprintln!("instance_pool sequence (pool, rebuilt): {rebuilds:?}");
    assert!(
        rebuilds.iter().skip(1).all(|(_, changed)| !changed),
        "the cache is recompiled whenever instance_pool changes: {rebuilds:?}"
    );
}
