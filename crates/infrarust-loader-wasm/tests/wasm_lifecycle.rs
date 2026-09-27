#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use infrarust_api::loader::LoaderError;
use infrarust_api::loader::PluginLoader;
use infrarust_api::plugin::PluginMetadata;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{fixture_path, fresh_loader};

const MARKER: &[u8] = b"LIFPROBE-BLOB-V1";
const BLOB_PAYLOAD: usize = 8192 - 16;
const DISCOVERY_BOUND: Duration = Duration::from_secs(30);

static COMPILE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

async fn compile_slot() -> tokio::sync::SemaphorePermit<'static> {
    COMPILE_SLOTS
        .acquire()
        .await
        .expect("the compile slots are never closed")
}

fn probe_bytes(settings: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture_path("lif-probe")).unwrap();
    let at = bytes
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .expect("lif-probe carries its settings blob")
        + MARKER.len();
    assert!(
        settings.len() < BLOB_PAYLOAD,
        "settings too long for the blob"
    );
    bytes[at..at + settings.len()].copy_from_slice(settings.as_bytes());
    bytes[at + settings.len()] = 0xFF;
    bytes
}

fn add_probe(plugins_dir: &Path, file_stem: &str, settings: &str) -> PathBuf {
    let path = plugins_dir.join(format!("{file_stem}.wasm"));
    std::fs::write(&path, probe_bytes(settings)).unwrap();
    path
}

async fn discover(plugins_dir: &Path) -> Result<Vec<PluginMetadata>, LoaderError> {
    let _slot = compile_slot().await;
    tokio::time::timeout(DISCOVERY_BOUND, fresh_loader().discover(plugins_dir))
        .await
        .expect("discovery finishes in bounded time")
}

async fn discover_logged(
    plugins_dir: &Path,
) -> (Result<Vec<PluginMetadata>, LoaderError>, LogCapture) {
    let logs = LogCapture::at(Level::ERROR);
    let found = discover(plugins_dir).with_subscriber(logs.clone()).await;
    (found, logs)
}

fn ids(metas: &[PluginMetadata]) -> Vec<&str> {
    let mut ids: Vec<&str> = metas.iter().map(|m| m.id.as_str()).collect();
    ids.sort_unstable();
    ids
}

async fn assert_good_plugin_survives(plugins_dir: &Path, what: &str) {
    let found = discover(plugins_dir).await;
    let metas = found.unwrap_or_else(|e| {
        panic!("a {what} in plugins_dir must not fail the discovery of the other plugins: {e}")
    });
    assert_eq!(ids(&metas), ["good"], "{what}");
}

type Artifacts = Vec<(std::ffi::OsString, Vec<u8>)>;

async fn add_precompiled_probe(plugins_dir: &Path, file_stem: &str, settings: &str) -> PathBuf {
    static COMPILED: std::sync::Mutex<
        std::collections::BTreeMap<String, std::sync::Arc<tokio::sync::OnceCell<Artifacts>>>,
    > = std::sync::Mutex::new(std::collections::BTreeMap::new());

    let path = add_probe(plugins_dir, file_stem, settings);
    let cell = std::sync::Arc::clone(
        COMPILED
            .lock()
            .unwrap()
            .entry(settings.to_owned())
            .or_default(),
    );
    let artifacts = cell
        .get_or_init(|| async {
            let _slot = compile_slot().await;
            let tmp = tempfile::tempdir().unwrap();
            add_probe(tmp.path(), file_stem, settings);
            fresh_loader().discover(tmp.path()).await.unwrap();
            std::fs::read_dir(tmp.path().join(".cache"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "cwasm"))
                .map(|path| {
                    (
                        path.file_name().unwrap().to_owned(),
                        std::fs::read(&path).unwrap(),
                    )
                })
                .collect()
        })
        .await;
    let cache = plugins_dir.join(".cache");
    std::fs::create_dir_all(&cache).unwrap();
    for (name, bytes) in artifacts {
        std::fs::write(cache.join(name), bytes).unwrap();
    }
    path
}

async fn staged_with_good() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_precompiled_probe(&dir, "good", "id=good\n").await;
    (tmp, dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_healthy_probe_is_discovered_with_its_patched_metadata() {
    let (_tmp, dir) = staged_with_good().await;
    let metas = discover(&dir).await.unwrap();
    assert_eq!(ids(&metas), ["good"]);
    assert_eq!(metas[0].version, "0.1.0");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zero_byte_wasm_does_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good().await;
    std::fs::write(dir.join("empty.wasm"), b"").unwrap();
    assert_good_plugin_survives(&dir, "zero-byte .wasm").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn random_bytes_do_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good().await;
    let noise: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(dir.join("noise.wasm"), noise).unwrap();
    assert_good_plugin_survives(&dir, "random-bytes .wasm").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_core_module_does_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good().await;
    std::fs::write(dir.join("core.wasm"), b"\0asm\x01\0\0\0").unwrap();
    assert_good_plugin_survives(&dir, "core module").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_component_does_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good().await;
    let bytes = probe_bytes("id=truncated\n");
    std::fs::write(dir.join("truncated.wasm"), &bytes[..bytes.len() / 2]).unwrap();
    assert_good_plugin_survives(&dir, "truncated component").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_refused_file_is_logged_once_with_its_path() {
    let (_tmp, dir) = staged_with_good().await;
    let empty = dir.join("empty.wasm");
    std::fs::write(&empty, b"").unwrap();
    let core = dir.join("core.wasm");
    std::fs::write(&core, b"\0asm\x01\0\0\0").unwrap();
    let panicker = add_probe(&dir, "panicker", "id=panicker\nmeta=panic\n");
    let (found, logs) = discover_logged(&dir).await;
    assert_eq!(ids(&found.unwrap()), ["good"]);
    let refusals = logs.matching("WASM plugin refused");
    assert_eq!(refusals.len(), 3, "{:?}", logs.lines());
    for path in [&empty, &core, &panicker] {
        let path = path.display().to_string();
        assert_eq!(
            refusals.iter().filter(|line| line.contains(&path)).count(),
            1,
            "{path}: {refusals:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_named_like_a_plugin_is_skipped() {
    let (_tmp, dir) = staged_with_good().await;
    std::fs::create_dir(dir.join("folder.wasm")).unwrap();
    assert_good_plugin_survives(&dir, "directory named x.wasm").await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_wasm_does_not_stop_the_other_plugins() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, dir) = staged_with_good().await;
    let locked = add_probe(&dir, "locked", "id=locked\n");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        return;
    }
    assert_good_plugin_survives(&dir, "unreadable .wasm").await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_dangling_symlink_does_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good().await;
    std::os::unix::fs::symlink(dir.join("missing-target"), dir.join("dangling.wasm")).unwrap();
    assert_good_plugin_survives(&dir, "dangling symlinked .wasm").await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_symlinked_wasm_is_loaded_once() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("plugins");
    std::fs::create_dir(&dir).unwrap();
    let outside = add_probe(tmp.path(), "linked", "id=linked\n");
    std::os::unix::fs::symlink(&outside, dir.join("linked.wasm")).unwrap();
    let metas = discover(&dir).await.unwrap();
    assert_eq!(ids(&metas), ["linked"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_file_in_a_subdirectory_is_ignored_and_not_logged() {
    let (_tmp, dir) = staged_with_good().await;
    std::fs::create_dir_all(dir.join("sub").join("deeper")).unwrap();
    add_probe(&dir.join("sub"), "nested", "id=nested\n");
    std::fs::write(dir.join("sub").join("deeper").join("junk.wasm"), b"").unwrap();
    std::fs::create_dir(dir.join("old")).unwrap();
    add_probe(&dir.join("old"), "good", "id=good\nversion=0.0.9\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
    #[cfg(unix)]
    let locked = {
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        add_probe(&locked, "hidden", "id=hidden\n");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        locked
    };

    let (found, logs) = discover_logged(&dir).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let metas = found.unwrap();
    assert_eq!(ids(&metas), ["good"], "{:?}", logs.lines());
    assert_eq!(
        metas[0].version, "0.1.0",
        "the top-level copy is the one loaded"
    );
    assert!(
        logs.lines().is_empty(),
        "nothing below plugins_dir is looked at, so nothing is refused: {:?}",
        logs.lines()
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_symlink_loop_among_the_plugin_files_is_refused_on_its_own() {
    let (_tmp, dir) = staged_with_good().await;
    let first = dir.join("ping.wasm");
    let second = dir.join("pong.wasm");
    std::os::unix::fs::symlink(&second, &first).unwrap();
    std::os::unix::fs::symlink(&first, &second).unwrap();
    let (found, logs) = discover_logged(&dir).await;
    assert_eq!(ids(&found.unwrap()), ["good"]);
    let refusals = logs.matching("WASM plugin refused");
    assert_eq!(refusals.len(), 2, "{:?}", logs.lines());
    for path in [&first, &second] {
        assert!(
            refusals
                .iter()
                .any(|line| line.contains(&path.display().to_string())),
            "{path:?}: {refusals:?}"
        );
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_file_reached_through_two_top_level_links_is_probed_once() {
    let (tmp, dir) = staged_with_good().await;
    std::os::unix::fs::symlink(dir.join("good.wasm"), dir.join("again.wasm")).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let linked = add_precompiled_probe(&outside, "linked", "id=linked\n").await;
    for artifact in std::fs::read_dir(outside.join(".cache")).unwrap() {
        let artifact = artifact.unwrap().path();
        std::fs::copy(
            &artifact,
            dir.join(".cache").join(artifact.file_name().unwrap()),
        )
        .unwrap();
    }
    std::os::unix::fs::symlink(&linked, dir.join("first-link.wasm")).unwrap();
    std::os::unix::fs::symlink(&linked, dir.join("second-link.wasm")).unwrap();
    let (found, logs) = discover_logged(&dir).await;
    assert_eq!(ids(&found.unwrap()), ["good", "linked"]);
    assert!(logs.lines().is_empty(), "{:?}", logs.lines());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_fifo_named_like_a_plugin_is_refused_without_blocking_discovery() {
    let (_tmp, dir) = staged_with_good().await;
    let fifo = dir.join("pipe.wasm");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|status| status.success()) {
        return;
    }
    let (found, logs) = discover_logged(&dir).await;
    assert_eq!(ids(&found.unwrap()), ["good"]);
    let refusals = logs.matching("not a regular file");
    assert_eq!(refusals.len(), 1, "{:?}", logs.lines());
    assert!(
        refusals[0].contains(&fifo.display().to_string()),
        "{refusals:?}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_non_utf8_file_name_is_loaded() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.wasm");
    std::fs::write(dir.join(name), probe_bytes("id=latin1\n")).unwrap();
    let metas = discover(&dir).await.unwrap();
    assert_eq!(ids(&metas), ["latin1"]);
}

const HUGE_SPARSE_LEN: u64 = 512 * 1024 * 1024;
const SHOWN_ERROR_BYTES: usize = 300;
const REFUSAL_BOUND: usize = 4096;

#[derive(Default)]
struct BoundedText {
    prefix: String,
    len: usize,
}

impl BoundedText {
    fn of(value: &impl std::fmt::Display) -> Self {
        use std::fmt::Write;
        let mut text = Self::default();
        let _ = write!(text, "{value}");
        text
    }
}

impl std::fmt::Write for BoundedText {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.len += s.len();
        for ch in s.chars() {
            if self.prefix.len() + ch.len_utf8() > SHOWN_ERROR_BYTES {
                break;
            }
            self.prefix.push(ch);
        }
        Ok(())
    }
}

type BoundedEvent = std::collections::BTreeMap<&'static str, BoundedText>;

#[derive(Clone, Default)]
struct BoundedLog {
    events: std::sync::Arc<std::sync::Mutex<Vec<BoundedEvent>>>,
}

impl BoundedLog {
    fn refusals_of(&self, path: &Path) -> Vec<(usize, usize, String)> {
        let path = path.display().to_string();
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                event
                    .get("message")
                    .is_some_and(|text| text.prefix == "WASM plugin refused")
                    && event.get("path").is_some_and(|text| text.prefix == path)
            })
            .map(|event| {
                let line = event.values().map(|text| text.len).sum();
                let error = event.get("error");
                (
                    line,
                    error.map_or(0, |text| text.len),
                    error.map(|text| text.prefix.clone()).unwrap_or_default(),
                )
            })
            .collect()
    }
}

struct BoundedFields<'a>(&'a mut BoundedEvent);

impl tracing::field::Visit for BoundedFields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let text = self.0.entry(field.name()).or_default();
        let _ = write!(text, "{value:?}");
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        use std::fmt::Write;
        let text = self.0.entry(field.name()).or_default();
        let _ = text.write_str(value);
    }
}

impl tracing::Subscriber for BoundedLog {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= Level::ERROR
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = BoundedEvent::new();
        event.record(&mut BoundedFields(&mut fields));
        self.events.lock().unwrap().push(fields);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn a_huge_non_component_is_refused_quickly() {
    let (_tmp, dir) = staged_with_good().await;
    let huge = dir.join("huge.wasm");
    let file = std::fs::File::create(&huge).unwrap();
    file.set_len(HUGE_SPARSE_LEN).unwrap();
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let allocated = std::fs::metadata(&huge).unwrap().blocks() * 512;
        assert!(
            allocated < 1024 * 1024,
            "huge.wasm must be a sparse file, yet {allocated} bytes are allocated on disk"
        );
    }
    let logs = BoundedLog::default();
    let started = Instant::now();
    let found = discover(&dir).with_subscriber(logs.clone()).await;
    let elapsed = started.elapsed();
    let error = found.as_ref().err().map(BoundedText::of);
    let error_len = error.as_ref().map_or(0, |text| text.len);
    let refusals = logs.refusals_of(&huge);
    eprintln!(
        "huge sparse .wasm: discovery took {elapsed:?}, ok={}, error message length={error_len}, refusal lines (length, error length)={:?}",
        found.is_ok(),
        refusals
            .iter()
            .map(|(line, error, _)| (*line, *error))
            .collect::<Vec<_>>()
    );
    let shown = error.map(|text| text.prefix).unwrap_or_default();
    assert!(
        error_len < REFUSAL_BOUND,
        "the refusal of one junk file carries a {error_len}-byte error message starting with: {shown:?}"
    );
    let metas =
        found.unwrap_or_else(|_| panic!("a 512 MiB junk file must not fail discovery: {shown:?}"));
    assert_eq!(ids(&metas), ["good"]);
    assert_eq!(
        refusals.len(),
        1,
        "huge.wasm is refused by one log line: {refusals:?}"
    );
    let (line, error, prefix) = &refusals[0];
    assert!(
        *line < REFUSAL_BOUND,
        "the refusal line of one junk file is {line} bytes long (error {error} bytes) and starts with: {prefix:?}"
    );
    assert!(
        prefix.contains("WebAssembly component"),
        "the refusal says the file is not a component: {prefix:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "a junk file is refused from its first bytes, yet discovery took {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn each_non_component_file_is_refused_with_its_own_reason() {
    let (_tmp, dir) = staged_with_good().await;
    let mut expected = vec![
        ("empty.wasm", b"".to_vec(), "the file is empty"),
        (
            "core.wasm",
            b"\0asm\x01\0\0\0".to_vec(),
            "core WebAssembly module",
        ),
        (
            "text.wasm",
            b"(component)\n".to_vec(),
            "text format is not accepted",
        ),
        (
            "elf.wasm",
            b"\x7fELF\x02\x01\x01\0\0\0\0\0".to_vec(),
            "magic bytes",
        ),
    ];
    for (name, bytes, _) in &expected {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = add_probe(&dir, "locked", "id=locked\n");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&locked).is_err() {
            expected.push(("locked.wasm", Vec::new(), "cannot read the plugin file"));
        } else {
            std::fs::remove_file(&locked).unwrap();
        }
    }
    let (found, logs) = discover_logged(&dir).await;
    assert_eq!(ids(&found.unwrap()), ["good"]);
    let refusals = logs.matching("WASM plugin refused");
    assert_eq!(refusals.len(), expected.len(), "{refusals:?}");
    for (name, _, reason) in &expected {
        let path = dir.join(name).display().to_string();
        let line = refusals
            .iter()
            .find(|line| line.contains(&path))
            .unwrap_or_else(|| panic!("{name} is not refused: {refusals:?}"));
        assert!(line.contains(reason), "{name}: {line}");
        assert!(!line.contains("directory"), "{name}: {line}");
    }
}

struct Managed {
    manager: infrarust_core::plugin::manager::PluginManager,
    discovery: Result<(), infrarust_core::plugin::PluginManagerError>,
    load_errors: Vec<String>,
}

impl Managed {
    fn state(&self, id: &str) -> Option<&infrarust_core::plugin::PluginState> {
        self.manager.plugin_state(id)
    }

    fn enabled(&self, id: &str) -> bool {
        self.manager.is_plugin_loaded(id)
    }
}

async fn manage(dir: &Path, proxy_toml: &str, extra: Vec<Box<dyn PluginLoader>>) -> Managed {
    let mut loaders = extra;
    loaders.push(Box::new(support::loader_from_toml(proxy_toml)));
    let mut manager = infrarust_core::plugin::manager::PluginManager::new(loaders);
    let _slot = compile_slot().await;
    let discovery = tokio::time::timeout(DISCOVERY_BOUND, manager.discover_all(dir))
        .await
        .expect("discovery finishes in bounded time")
        .map(|_| ());
    let mut load_errors = Vec::new();
    if discovery.is_ok() {
        let factory = std::sync::Arc::new(support::make_env(dir.to_path_buf()).factory);
        load_errors = manager
            .load_and_enable_all(factory)
            .await
            .into_iter()
            .map(|e| e.to_string())
            .collect();
    }
    Managed {
        manager,
        discovery,
        load_errors,
    }
}

fn native_loader(id: &str) -> Box<dyn PluginLoader> {
    let loader = infrarust_core::plugin::StaticPluginLoader::new();
    let owned = id.to_owned();
    loader.register(PluginMetadata::new(id, id, "1.0.0"), move || {
        Box::new(infrarust_test_harness::ScriptedPlugin::new(owned.clone()))
    });
    Box::new(loader)
}

fn log_lines(dir: &Path, id: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(id).join("log.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn stamp_of(dir: &Path, id: &str, prefix: &str) -> u128 {
    log_lines(dir, id)
        .iter()
        .find(|line| line.starts_with(prefix))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|stamp| stamp.parse().ok())
        .unwrap_or_else(|| panic!("{id} logged no `{prefix}` line: {:?}", log_lines(dir, id)))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zero_byte_wasm_does_not_stop_the_plugin_manager() {
    let (_tmp, dir) = staged_with_good().await;
    std::fs::write(dir.join("empty.wasm"), b"").unwrap();
    let managed = manage(&dir, "", Vec::new()).await;
    assert!(
        managed.discovery.is_ok(),
        "one bad file must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("good"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_proxy_starts_with_its_good_plugins_when_plugins_dir_holds_a_zero_byte_wasm() {
    let (_tmp, dir) = staged_with_good().await;
    std::fs::write(dir.join("empty.wasm"), b"").unwrap();
    let plugins_dir = dir.clone();
    let _slot = compile_slot().await;
    let started = infrarust_test_harness::TestProxy::builder()
        .loader(Box::new(fresh_loader()))
        .patch_config(move |table| {
            table.insert(
                "plugins_dir".into(),
                toml::Value::String(plugins_dir.to_string_lossy().into_owned()),
            );
        })
        .start()
        .await;
    let proxy = started.unwrap_or_else(|e| {
        panic!("a zero-byte .wasm in plugins_dir must not stop the proxy from starting: {e}")
    });
    assert!(
        proxy.plugin_context("good").await.is_some(),
        "the good plugin is enabled"
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_hard_dependency_fails_only_the_plugin_that_needs_it() {
    let (_tmp, dir) = staged_with_good().await;
    add_precompiled_probe(&dir, "needy", "id=needy\ndep=absent\n").await;
    let managed = manage(&dir, "", Vec::new()).await;
    assert!(
        managed.discovery.is_ok(),
        "a missing dependency of one plugin must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("good"));
    assert!(!managed.enabled("needy"));
    let refusal = "plugin 'needy' requires 'absent', which was not found";
    assert!(
        matches!(managed.state("needy"), Some(infrarust_core::plugin::PluginState::Error(e)) if e == refusal),
        "{:?}",
        managed.state("needy")
    );
    assert_eq!(managed.load_errors, [refusal]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_optional_dependency_is_ignored() {
    let (_tmp, dir) = staged_with_good().await;
    add_precompiled_probe(&dir, "relaxed", "id=relaxed\nsoftdep=absent\n").await;
    let managed = manage(&dir, "", Vec::new()).await;
    managed.discovery.as_ref().unwrap();
    assert!(managed.enabled("good") && managed.enabled("relaxed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dependency_cycle_fails_only_the_plugins_in_the_cycle() {
    let (_tmp, dir) = staged_with_good().await;
    add_precompiled_probe(&dir, "ping", "id=ping\ndep=pong\n").await;
    add_precompiled_probe(&dir, "pong", "id=pong\ndep=ping\n").await;
    let managed = manage(&dir, "", Vec::new()).await;
    assert!(
        managed.discovery.is_ok(),
        "a cycle between two plugins must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("good"));
    assert!(!managed.enabled("ping") && !managed.enabled("pong"));
    assert_eq!(
        managed.load_errors,
        [
            "plugin 'ping' is refused: its dependencies form a cycle (ping, pong)",
            "plugin 'pong' is refused: its dependencies form a cycle (ping, pong)",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_wasm_files_with_the_same_id_do_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good().await;
    add_precompiled_probe(&dir, "twin-a", "id=twin\n").await;
    add_precompiled_probe(&dir, "twin-b", "id=twin\nversion=0.0.9\n").await;
    let managed = manage(&dir, "", Vec::new()).await;
    assert!(
        managed.discovery.is_ok(),
        "a duplicated plugin file must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("good"));
    assert!(!managed.enabled("twin"), "neither copy of twin is enabled");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_wasm_files_with_the_same_id_are_refused_by_one_error_naming_both() {
    let (_tmp, dir) = staged_with_good().await;
    let first = add_precompiled_probe(&dir, "twin-a", "id=twin\n").await;
    let second = add_precompiled_probe(&dir, "twin-b", "id=twin\nversion=0.0.9\n").await;
    let (found, logs) = discover_logged(&dir).await;
    assert_eq!(ids(&found.unwrap()), ["good"]);
    let refusals = logs.matching("same plugin id");
    assert_eq!(refusals.len(), 1, "{:?}", logs.lines());
    for path in [&first, &second] {
        assert!(
            refusals[0].contains(&path.display().to_string()),
            "{path:?}: {refusals:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_id_colliding_with_a_native_plugin_does_not_stop_the_native_one() {
    let (_tmp, dir) = staged_with_good().await;
    let impostor = add_precompiled_probe(&dir, "impostor", "id=native-core\n").await;
    let managed = manage(&dir, "", vec![native_loader("native-core")]).await;
    assert!(
        managed.discovery.is_ok(),
        "a WASM file reusing a native plugin id must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("native-core") && managed.enabled("good"));
    assert_eq!(
        managed.load_errors,
        [format!(
            "plugin 'native-core' from loader 'wasm' ({}) is refused: loader 'static' already provides that id",
            impostor.display()
        )]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_whose_hard_dependency_failed_to_enable_is_not_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_precompiled_probe(&dir, "base", "id=base\nenable=fail\n").await;
    add_precompiled_probe(&dir, "dependent", "id=dependent\ndep=base\n").await;
    let managed = manage(&dir, "", Vec::new()).await;
    managed.discovery.as_ref().unwrap();
    assert!(!managed.enabled("base"), "base refused its own enable");
    assert!(
        !managed.enabled("dependent"),
        "dependent requires base, which is not enabled; state = {:?}, errors = {:?}",
        managed.state("dependent"),
        managed.load_errors
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_depending_on_a_config_disabled_plugin_is_not_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_precompiled_probe(&dir, "base", "id=base\n").await;
    add_precompiled_probe(&dir, "dependent", "id=dependent\ndep=base\n").await;
    let loader = support::loader_from_toml("");
    let mut manager = infrarust_core::plugin::manager::PluginManager::new(vec![Box::new(loader)]);
    manager.set_disabled_plugins(std::collections::HashSet::from(["base".to_owned()]));
    let slot = compile_slot().await;
    manager.discover_all(&dir).await.unwrap();
    drop(slot);
    let factory = std::sync::Arc::new(support::make_env(dir.clone()).factory);
    manager.load_and_enable_all(factory).await;
    assert!(!manager.is_plugin_loaded("base"));
    assert!(
        !manager.is_plugin_loaded("dependent"),
        "dependent requires base, which enabled = false keeps off; state = {:?}",
        manager.plugin_state("dependent")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_plugin_depending_on_a_native_plugin_enables_after_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_precompiled_probe(&dir, "rider", "id=rider\ndep=native-core\n").await;
    let managed = manage(&dir, "", vec![native_loader("native-core")]).await;
    managed.discovery.as_ref().unwrap();
    assert!(managed.enabled("native-core") && managed.enabled("rider"));
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_disables_a_dependent_before_its_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_precompiled_probe(&dir, "zz-base", "id=zz-base\n").await;
    add_precompiled_probe(&dir, "aa-top", "id=aa-top\ndep=mid\n").await;
    add_precompiled_probe(&dir, "mid", "id=mid\ndep=zz-base\n").await;
    let mut managed = manage(&dir, "", Vec::new()).await;
    managed.discovery.as_ref().unwrap();
    assert!(managed.enabled("aa-top"));
    assert!(stamp_of(&dir, "zz-base", "enable-end") < stamp_of(&dir, "mid", "enable "));
    assert!(stamp_of(&dir, "mid", "enable-end") < stamp_of(&dir, "aa-top", "enable "));
    managed.manager.shutdown().await;
    assert!(stamp_of(&dir, "aa-top", "disable-end") < stamp_of(&dir, "mid", "disable "));
    assert!(stamp_of(&dir, "mid", "disable-end") < stamp_of(&dir, "zz-base", "disable "));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sleeping_metadata_export_does_not_hang_discovery() {
    let (_tmp, dir) = staged_with_good().await;
    add_probe(&dir, "sleeper", "id=sleeper\nmeta=sleep:3600000\n");
    let loader = support::loader_from_toml("[wasm]\nmax_call_duration = \"2s\"\n");
    let _slot = compile_slot().await;
    let started = Instant::now();
    let outcome = tokio::time::timeout(Duration::from_secs(20), loader.discover(&dir)).await;
    let elapsed = started.elapsed();
    let found = outcome.unwrap_or_else(|_| {
        panic!("discovery still blocked after {elapsed:?} on a metadata() that sleeps; max_call_duration is 2s")
    });
    let metas =
        found.unwrap_or_else(|e| panic!("the sleeper must not fail discovery of the others: {e}"));
    assert_eq!(ids(&metas), ["good"]);
}

async fn discover_with_a_sleeper(proxy_toml: &str, warm_first: bool) -> (Duration, LogCapture) {
    let (_tmp, dir) = staged_with_good().await;
    add_probe(&dir, "sleeper", "id=sleeper\nmeta=sleep:3600000\n");
    let loader = support::loader_from_toml(proxy_toml);
    let _slot = compile_slot().await;
    if warm_first {
        loader.discover(&dir).await.unwrap();
    }
    let logs = LogCapture::at(Level::ERROR);
    let started = Instant::now();
    let found = tokio::time::timeout(DISCOVERY_BOUND, loader.discover(&dir))
        .with_subscriber(logs.clone())
        .await
        .expect("discovery finishes in bounded time");
    let elapsed = started.elapsed();
    assert_eq!(ids(&found.unwrap()), ["good"]);
    (elapsed, logs)
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_is_cut_at_max_call_duration_when_that_is_shorter_than_five_seconds() {
    let (elapsed, logs) =
        discover_with_a_sleeper("[wasm]\nmax_call_duration = \"1s\"\n", true).await;
    assert!(
        elapsed >= Duration::from_secs(1) && elapsed < Duration::from_secs(4),
        "discovery with a cached sleeper took {elapsed:?}, the metadata limit is 1s"
    );
    let refusals = logs.matching("metadata() did not return within 1s");
    assert_eq!(refusals.len(), 1, "{:?}", logs.lines());
    assert!(refusals[0].contains("sleeper.wasm"), "{refusals:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_is_cut_at_five_seconds_under_the_default_max_call_duration() {
    let (elapsed, logs) = discover_with_a_sleeper("", false).await;
    assert!(
        elapsed >= Duration::from_secs(5),
        "the sleeper was cut after {elapsed:?}, before the 5s metadata limit"
    );
    let refusals = logs.matching("metadata() did not return within 5s");
    assert_eq!(refusals.len(), 1, "{:?}", logs.lines());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spinning_metadata_export_is_cut_and_does_not_stop_the_others() {
    let (_tmp, dir) = staged_with_good().await;
    add_probe(&dir, "spinner", "id=spinner\nmeta=spin\n");
    let started = Instant::now();
    let found = discover(&dir).await;
    eprintln!("spinning metadata: discovery took {:?}", started.elapsed());
    let metas =
        found.unwrap_or_else(|e| panic!("the spinner must not fail discovery of the others: {e}"));
    assert_eq!(ids(&metas), ["good"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_metadata_export_does_not_stop_the_others() {
    let (_tmp, dir) = staged_with_good().await;
    add_probe(&dir, "panicker", "id=panicker\nmeta=panic\n");
    let metas = discover(&dir)
        .await
        .unwrap_or_else(|e| panic!("the panicker must not fail discovery of the others: {e}"));
    assert_eq!(ids(&metas), ["good"]);
}

async fn load_probe_with_id(id_settings: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir(&plugins_dir).unwrap();
    add_probe(&plugins_dir, "probe", id_settings);
    let loader = fresh_loader();
    let logs = LogCapture::at(Level::ERROR);
    let slot = compile_slot().await;
    let metas = tokio::time::timeout(DISCOVERY_BOUND, loader.discover(&plugins_dir))
        .with_subscriber(logs.clone())
        .await
        .map_err(|_| "discover hung".to_owned())?
        .map_err(|e| format!("discover: {e}"))?;
    drop(slot);
    let id = metas.iter().map(|m| m.id.clone()).next().ok_or_else(|| {
        format!(
            "refused at discovery: {}",
            logs.matching("WASM plugin refused").join(" | ")
        )
    })?;
    let env = support::make_env(plugins_dir.clone());
    let plugin = loader
        .load(&id, &env.factory)
        .await
        .map_err(|e| format!("load: {e}"))?;
    let ctx = env.factory.context(&id);
    tokio::time::timeout(Duration::from_secs(10), plugin.on_enable(ctx.as_ref()))
        .await
        .map_err(|_| "on_enable hung".to_owned())?
        .map_err(|e| format!("on_enable: {e}"))?;
    Ok((tmp, plugins_dir))
}

fn escapes(root: &Path, plugins_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.starts_with(plugins_dir) {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("log.txt") {
                out.push(path.clone());
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    out
}

fn assert_refused_by_the_id_rule(
    outcome: Result<(tempfile::TempDir, PathBuf), String>,
    expected: &str,
) {
    match outcome {
        Ok((tmp, plugins_dir)) => panic!(
            "the host loaded a plugin whose id breaks the #[plugin] rule [a-z0-9][a-z0-9_-]{{0,63}}; log.txt files outside plugins_dir: {:?}",
            escapes(tmp.path(), &plugins_dir)
        ),
        Err(reason) => assert!(
            reason.contains("refused at discovery") && reason.contains(expected),
            "expected a discovery refusal saying {expected:?}, got: {reason}"
        ),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dot_dot_plugin_id_cannot_write_outside_plugins_dir() {
    let outcome = load_probe_with_id("id=../escape\n").await;
    assert_refused_by_the_id_rule(
        outcome,
        "plugin id `../escape` must start with a lowercase letter or a digit",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dot_dot_inside_a_plugin_id_is_refused() {
    let outcome = load_probe_with_id("id=up/../../escape\n").await;
    assert_refused_by_the_id_rule(outcome, "plugin id `up/../../escape` contains `/`");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_absolute_plugin_id_cannot_redirect_the_data_dir() {
    let target = tempfile::tempdir().unwrap();
    let stolen = target.path().join("stolen");
    let outcome = load_probe_with_id(&format!("id={}\n", stolen.display())).await;
    assert!(
        escapes(target.path(), Path::new("/nonexistent-plugins")).is_empty(),
        "an absolute plugin id redirected the data dir outside plugins_dir"
    );
    assert_refused_by_the_id_rule(outcome, "must start with a lowercase letter or a digit");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dot_prefixed_plugin_id_cannot_take_the_cache_directory() {
    let outcome = load_probe_with_id("id=.cache\n").await;
    assert_refused_by_the_id_rule(outcome, "plugin id `.cache` must start with");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_plugin_id_is_refused() {
    let outcome = load_probe_with_id("id=\n").await;
    assert_refused_by_the_id_rule(outcome, "a plugin id cannot be empty");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_unicode_plugin_id_is_refused_like_the_plugin_macro_refuses_it() {
    let outcome = load_probe_with_id("id=café-plugin\n").await;
    assert_refused_by_the_id_rule(outcome, "plugin id `café-plugin` contains `é`");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_id_longer_than_64_characters_is_refused() {
    let id = "a".repeat(300);
    let outcome = load_probe_with_id(&format!("id={id}\n")).await;
    assert_refused_by_the_id_rule(outcome, "is longer than 64 characters");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_64_character_plugin_id_is_loaded() {
    let id = "a".repeat(64);
    let (_tmp, plugins_dir) = load_probe_with_id(&format!("id={id}\n"))
        .await
        .unwrap_or_else(|reason| panic!("a 64-character id is inside the rule: {reason}"));
    assert!(plugins_dir.join(&id).is_dir());
}
