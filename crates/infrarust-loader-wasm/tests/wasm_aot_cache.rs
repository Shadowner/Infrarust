#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use infrarust_api::loader::{LoaderError, PluginContextFactory, PluginLoader};
use infrarust_api::plugin::PluginMetadata;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{
    add_fixture, cache_dir_of, cached_loader, cached_loader_from_toml, fixture_path,
    loader_from_toml, make_env,
};

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
    discover_with(&cached_loader(dir), dir).await
}

fn ids(metas: &[PluginMetadata]) -> Vec<String> {
    let mut ids: Vec<String> = metas.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    ids
}

fn cache_entries(dir: &Path) -> Vec<PathBuf> {
    entries_in(&cache_dir_of(dir))
}

fn entries_in(cache: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(cache)
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

    let loader = cached_loader(&dir);
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

fn plugins_and_cache() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    std::fs::create_dir(&plugins).unwrap();
    let cache = tmp.path().join("state").join("cache").join("wasm");
    (tmp, plugins, cache)
}

fn loader_with_cache_dir(cache: &Path, extra: &str) -> infrarust_loader_wasm::WasmPluginLoader {
    loader_from_toml(&format!(
        "[wasm]\ncache_dir = {:?}\n{extra}",
        cache.display().to_string()
    ))
}

async fn swapped_in_artifact() -> Vec<u8> {
    let other = tempfile::tempdir().unwrap();
    add_probe(other.path(), "other", "id=swapped-in\n");
    assert_eq!(ids(&discover(other.path()).await.unwrap()), ["swapped-in"]);
    std::fs::read(only_cwasm(other.path())).unwrap()
}

fn only_entry_in(cache: &Path) -> PathBuf {
    let entries: Vec<PathBuf> = entries_in(cache)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "cwasm"))
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    entries[0].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_artifact_planted_in_plugins_dir_is_never_run() {
    let (_tmp, plugins, cache) = plugins_and_cache();
    add_probe(&plugins, "good", "id=good\n");
    let found = discover_with(&loader_with_cache_dir(&cache, ""), &plugins)
        .await
        .unwrap();
    assert_eq!(ids(&found), ["good"]);
    let entry = only_entry_in(&cache);
    assert!(
        !plugins.join(".cache").exists(),
        "nothing is written under plugins_dir"
    );

    let planted = plugins.join(".cache");
    std::fs::create_dir(&planted).unwrap();
    std::fs::write(
        planted.join(entry.file_name().unwrap()),
        swapped_in_artifact().await,
    )
    .unwrap();
    std::fs::remove_file(&entry).unwrap();

    let logs = LogCapture::at(Level::INFO);
    let loader = loader_with_cache_dir(&cache, "");
    for _ in 0..2 {
        let found = discover_with(&loader, &plugins)
            .with_subscriber(logs.clone())
            .await
            .unwrap();
        assert_eq!(
            ids(&found),
            ["good"],
            "good.wasm runs its own code, not an artifact left under plugins_dir"
        );
    }
    assert_eq!(
        logs.matching("no longer read and can be deleted").len(),
        1,
        "{:?}",
        logs.lines()
    );
    assert!(planted.join(entry.file_name().unwrap()).exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_cache_entry_writable_by_group_or_others_is_refused_and_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, plugins, cache) = plugins_and_cache();
    add_probe(&plugins, "good", "id=good\n");
    discover_with(&loader_with_cache_dir(&cache, ""), &plugins)
        .await
        .unwrap();
    let entry = only_entry_in(&cache);
    let original = std::fs::read(&entry).unwrap();
    assert_eq!(
        std::fs::metadata(&entry).unwrap().permissions().mode() & 0o077,
        0,
        "an entry is written readable by the proxy user only"
    );

    std::fs::write(&entry, swapped_in_artifact().await).unwrap();
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o664)).unwrap();
    let logs = LogCapture::at(Level::WARN);
    let found = discover_with(&loader_with_cache_dir(&cache, ""), &plugins)
        .with_subscriber(logs.clone())
        .await
        .unwrap();
    assert_eq!(
        ids(&found),
        ["good"],
        "a group-writable entry is not loaded"
    );
    let refusals = logs.matching("AOT cache entry refused");
    assert_eq!(refusals.len(), 1, "{:?}", logs.lines());
    assert!(
        refusals[0].contains("writable by group or others"),
        "{refusals:?}"
    );
    assert_eq!(std::fs::read(&entry).unwrap(), original);
    assert_eq!(
        std::fs::metadata(&entry).unwrap().permissions().mode() & 0o022,
        0
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_symlinked_cache_entry_is_not_followed() {
    let (tmp, plugins, cache) = plugins_and_cache();
    add_probe(&plugins, "good", "id=good\n");
    discover_with(&loader_with_cache_dir(&cache, ""), &plugins)
        .await
        .unwrap();
    let entry = only_entry_in(&cache);
    let original = std::fs::read(&entry).unwrap();

    let elsewhere = tmp.path().join("elsewhere.cwasm");
    std::fs::write(&elsewhere, swapped_in_artifact().await).unwrap();
    std::fs::remove_file(&entry).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &entry).unwrap();
    let logs = LogCapture::at(Level::WARN);
    let found = discover_with(&loader_with_cache_dir(&cache, ""), &plugins)
        .with_subscriber(logs.clone())
        .await
        .unwrap();
    assert_eq!(ids(&found), ["good"]);
    assert_eq!(
        logs.matching("not a regular file").len(),
        1,
        "{:?}",
        logs.lines()
    );
    assert!(!std::fs::symlink_metadata(&entry).unwrap().is_symlink());
    assert_eq!(std::fs::read(&entry).unwrap(), original);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_cache_dir_does_not_stop_loading() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, plugins, cache) = plugins_and_cache();
    add_probe(&plugins, "good", "id=good\n");
    add_probe(&plugins, "second", "id=second\n");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(cache.join("probe")).is_ok() {
        return;
    }
    let logs = LogCapture::at(Level::WARN);
    let found = discover_with(&loader_with_cache_dir(&cache, ""), &plugins)
        .with_subscriber(logs.clone())
        .await;
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755)).unwrap();
    let metas = found.unwrap_or_else(|e| {
        panic!("an unwritable cache must degrade to in-memory compilation, not fail: {e}")
    });
    assert_eq!(ids(&metas), ["good", "second"]);
    let warnings = logs.matching("AOT cache directory cannot be written");
    assert_eq!(warnings.len(), 1, "one warning: {:?}", logs.lines());
    assert!(
        warnings[0].contains(&cache.display().to_string()),
        "{warnings:?}"
    );
    assert!(entries_in(&cache).is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_cache_dir_that_cannot_be_created_does_not_stop_loading() {
    use std::os::unix::fs::PermissionsExt;
    let (tmp, plugins, _) = plugins_and_cache();
    add_probe(&plugins, "good", "id=good\n");
    let locked = tmp.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(locked.join("probe")).is_ok() {
        return;
    }
    let found = discover_with(
        &loader_with_cache_dir(&locked.join("cache").join("wasm"), ""),
        &plugins,
    )
    .await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(ids(&found.unwrap()), ["good"]);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_plugins_dir_still_loads_its_plugins() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, plugins, cache) = plugins_and_cache();
    add_probe(&plugins, "good", "id=good\n");
    std::fs::set_permissions(&plugins, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(plugins.join("probe")).is_ok() {
        return;
    }
    let found = discover_with(&loader_with_cache_dir(&cache, ""), &plugins).await;
    std::fs::set_permissions(&plugins, std::fs::Permissions::from_mode(0o755)).unwrap();
    let metas = found.unwrap_or_else(|e| {
        panic!("a read-only plugins_dir (for example a mounted volume) must still load: {e}")
    });
    assert_eq!(ids(&metas), ["good"]);
    only_entry_in(&cache);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_loaders_sharing_one_cache_dir_all_succeed() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "good", "id=good\n");
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let dir = dir.clone();
        tasks.push(tokio::spawn(async move {
            let loader = cached_loader(&dir);
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
        "temporary files left in the cache directory: {leftovers:?}"
    );
    assert_eq!(cwasm_entries(&dir).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
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
        "one plugin updated {updates} times leaves {} artifacts ({bytes} bytes) in the cache",
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
        let loader = cached_loader_from_toml(&format!("[wasm]\ninstance_pool = {pool}\n"), &dir);
        let before = std::fs::metadata(cache_dir_of(&dir)).ok().and_then(|_| {
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

fn leb128(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
    out.push(id);
    leb128(u32::try_from(body.len()).unwrap(), out);
    out.extend_from_slice(body);
}

fn component_with_a_table_of(elements: u32) -> Vec<u8> {
    let mut tables = vec![1, 0x70, 1];
    leb128(elements, &mut tables);
    leb128(elements, &mut tables);
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    section(4, &tables, &mut module);
    let mut component = b"\0asm\x0d\0\x01\0".to_vec();
    section(1, &module, &mut component);
    section(2, &[1, 0, 0, 0], &mut component);
    component
}

#[tokio::test(flavor = "multi_thread")]
async fn a_component_the_pool_cannot_hold_keeps_its_cache_entry_and_names_the_pool_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    std::fs::write(dir.join("wide.wasm"), component_with_a_table_of(20_000)).unwrap();
    let pooled = || cached_loader_from_toml("[wasm]\ninstance_pool = 8\n", &dir);
    let logs = LogCapture::at(Level::WARN);

    let found = discover_with(&pooled(), &dir)
        .with_subscriber(logs.clone())
        .await
        .unwrap();
    assert!(found.is_empty(), "{found:?}");
    let entry = only_cwasm(&dir);
    let written = std::fs::metadata(&entry).unwrap().modified().unwrap();
    discover_with(&pooled(), &dir)
        .with_subscriber(logs.clone())
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&entry).unwrap().modified().unwrap(),
        written,
        "an entry the pool cannot load is kept, not rewritten"
    );
    let refusals = logs.matching("WASM plugin refused");
    assert_eq!(refusals.len(), 2, "{:?}", logs.lines());
    assert!(
        refusals
            .iter()
            .all(|line| line.contains("exceeds the limit of")),
        "the refusal names the pooled table limit: {refusals:?}"
    );
    assert!(
        logs.matching("replaced by a fresh compilation").is_empty(),
        "{:?}",
        logs.lines()
    );

    discover_with(&cached_loader(&dir), &dir)
        .with_subscriber(logs.clone())
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&entry).unwrap().modified().unwrap(),
        written,
        "without a pool the same entry loads"
    );
    assert_eq!(
        logs.matching("not an Infrarust plugin").len(),
        1,
        "{:?}",
        logs.lines()
    );
}
