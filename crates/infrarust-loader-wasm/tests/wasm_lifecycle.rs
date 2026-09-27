#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use infrarust_api::loader::PluginLoader;
use infrarust_api::plugin::PluginMetadata;
use infrarust_api::loader::LoaderError;

use support::{fixture_path, fresh_loader};

const MARKER: &[u8] = b"LIFPROBE-BLOB-V1";
const BLOB_PAYLOAD: usize = 8192 - 16;
const DISCOVERY_BOUND: Duration = Duration::from_secs(30);

fn probe_bytes(settings: &str) -> Vec<u8> {
    let mut bytes = std::fs::read(fixture_path("lif-probe")).unwrap();
    let at = bytes
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .expect("lif-probe carries its settings blob")
        + MARKER.len();
    assert!(settings.len() < BLOB_PAYLOAD, "settings too long for the blob");
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
    tokio::time::timeout(DISCOVERY_BOUND, fresh_loader().discover(plugins_dir))
        .await
        .expect("discovery finishes in bounded time")
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

fn staged_with_good() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "good", "id=good\n");
    (tmp, dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_healthy_probe_is_discovered_with_its_patched_metadata() {
    let (_tmp, dir) = staged_with_good();
    let metas = discover(&dir).await.unwrap();
    assert_eq!(ids(&metas), ["good"]);
    assert_eq!(metas[0].version, "0.1.0");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zero_byte_wasm_does_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good();
    std::fs::write(dir.join("empty.wasm"), b"").unwrap();
    assert_good_plugin_survives(&dir, "zero-byte .wasm").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn random_bytes_do_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good();
    let noise: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    std::fs::write(dir.join("noise.wasm"), noise).unwrap();
    assert_good_plugin_survives(&dir, "random-bytes .wasm").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_core_module_does_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good();
    std::fs::write(dir.join("core.wasm"), b"\0asm\x01\0\0\0").unwrap();
    assert_good_plugin_survives(&dir, "core module").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_component_does_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good();
    let bytes = probe_bytes("id=truncated\n");
    std::fs::write(dir.join("truncated.wasm"), &bytes[..bytes.len() / 2]).unwrap();
    assert_good_plugin_survives(&dir, "truncated component").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_named_like_a_plugin_is_skipped() {
    let (_tmp, dir) = staged_with_good();
    std::fs::create_dir(dir.join("folder.wasm")).unwrap();
    assert_good_plugin_survives(&dir, "directory named x.wasm").await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_wasm_does_not_stop_the_other_plugins() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, dir) = staged_with_good();
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
    let (_tmp, dir) = staged_with_good();
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

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_directory_symlink_loop_does_not_stop_discovery() {
    let (_tmp, dir) = staged_with_good();
    std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
    assert_good_plugin_survives(&dir, "symlink loop to plugins_dir").await;
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

#[tokio::test(flavor = "multi_thread")]
async fn a_huge_non_component_is_refused_quickly() {
    let (_tmp, dir) = staged_with_good();
    let file = std::fs::File::create(dir.join("huge.wasm")).unwrap();
    file.set_len(512 * 1024 * 1024).unwrap();
    drop(file);
    let started = Instant::now();
    let found = discover(&dir).await;
    let elapsed = started.elapsed();
    let error_len = found.as_ref().err().map_or(0, |e| e.to_string().len());
    eprintln!("huge sparse .wasm: discovery took {elapsed:?}, ok={}, error message length={error_len}", found.is_ok());
    assert!(error_len < 4096, "the refusal of one junk file carries a {error_len}-byte error message");
    let metas = found.unwrap_or_else(|e| {
        let shown: String = e.to_string().chars().take(300).collect();
        panic!("a 512 MiB junk file must not fail discovery: {shown}")
    });
    assert_eq!(ids(&metas), ["good"]);
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

async fn manage(
    dir: &Path,
    proxy_toml: &str,
    extra: Vec<Box<dyn PluginLoader>>,
) -> Managed {
    let mut loaders = extra;
    loaders.push(Box::new(support::loader_from_toml(proxy_toml)));
    let mut manager = infrarust_core::plugin::manager::PluginManager::new(loaders);
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
    let (_tmp, dir) = staged_with_good();
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
    let (_tmp, dir) = staged_with_good();
    std::fs::write(dir.join("empty.wasm"), b"").unwrap();
    let plugins_dir = dir.clone();
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
    assert!(proxy.plugin_context("good").await.is_some(), "the good plugin is enabled");
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_hard_dependency_fails_only_the_plugin_that_needs_it() {
    let (_tmp, dir) = staged_with_good();
    add_probe(&dir, "needy", "id=needy\ndep=absent\n");
    let managed = manage(&dir, "", Vec::new()).await;
    assert!(
        managed.discovery.is_ok(),
        "a missing dependency of one plugin must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("good"));
    assert!(!managed.enabled("needy"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_optional_dependency_is_ignored() {
    let (_tmp, dir) = staged_with_good();
    add_probe(&dir, "relaxed", "id=relaxed\nsoftdep=absent\n");
    let managed = manage(&dir, "", Vec::new()).await;
    managed.discovery.as_ref().unwrap();
    assert!(managed.enabled("good") && managed.enabled("relaxed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dependency_cycle_fails_only_the_plugins_in_the_cycle() {
    let (_tmp, dir) = staged_with_good();
    add_probe(&dir, "ping", "id=ping\ndep=pong\n");
    add_probe(&dir, "pong", "id=pong\ndep=ping\n");
    let managed = manage(&dir, "", Vec::new()).await;
    assert!(
        managed.discovery.is_ok(),
        "a cycle between two plugins must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("good"));
    assert!(!managed.enabled("ping") && !managed.enabled("pong"));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_wasm_files_with_the_same_id_do_not_stop_the_other_plugins() {
    let (_tmp, dir) = staged_with_good();
    add_probe(&dir, "twin-a", "id=twin\n");
    std::fs::create_dir(dir.join("backup")).unwrap();
    add_probe(&dir.join("backup"), "twin-b", "id=twin\nversion=0.0.9\n");
    let managed = manage(&dir, "", Vec::new()).await;
    assert!(
        managed.discovery.is_ok(),
        "a duplicated plugin file must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("good"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_id_colliding_with_a_native_plugin_does_not_stop_the_native_one() {
    let (_tmp, dir) = staged_with_good();
    add_probe(&dir, "impostor", "id=native-core\n");
    let managed = manage(&dir, "", vec![native_loader("native-core")]).await;
    assert!(
        managed.discovery.is_ok(),
        "a WASM file reusing a native plugin id must not fail discover_all: {:?}",
        managed.discovery
    );
    assert!(managed.enabled("native-core") && managed.enabled("good"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_whose_hard_dependency_failed_to_enable_is_not_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "base", "id=base\nenable=fail\n");
    add_probe(&dir, "dependent", "id=dependent\ndep=base\n");
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
    add_probe(&dir, "base", "id=base\n");
    add_probe(&dir, "dependent", "id=dependent\ndep=base\n");
    let loader = support::loader_from_toml("");
    let mut manager =
        infrarust_core::plugin::manager::PluginManager::new(vec![Box::new(loader)]);
    manager.set_disabled_plugins(std::collections::HashSet::from(["base".to_owned()]));
    manager.discover_all(&dir).await.unwrap();
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
    add_probe(&dir, "rider", "id=rider\ndep=native-core\n");
    let managed = manage(&dir, "", vec![native_loader("native-core")]).await;
    managed.discovery.as_ref().unwrap();
    assert!(managed.enabled("native-core") && managed.enabled("rider"));
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_disables_a_dependent_before_its_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    add_probe(&dir, "zz-base", "id=zz-base\n");
    add_probe(&dir, "aa-top", "id=aa-top\ndep=mid\n");
    add_probe(&dir, "mid", "id=mid\ndep=zz-base\n");
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
    let (_tmp, dir) = staged_with_good();
    add_probe(&dir, "sleeper", "id=sleeper\nmeta=sleep:3600000\n");
    let loader = support::loader_from_toml("[wasm]\nmax_call_duration = \"2s\"\n");
    let started = Instant::now();
    let outcome = tokio::time::timeout(Duration::from_secs(20), loader.discover(&dir)).await;
    let elapsed = started.elapsed();
    let found = outcome.unwrap_or_else(|_| {
        panic!("discovery still blocked after {elapsed:?} on a metadata() that sleeps; max_call_duration is 2s")
    });
    let metas = found.unwrap_or_else(|e| panic!("the sleeper must not fail discovery of the others: {e}"));
    assert_eq!(ids(&metas), ["good"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spinning_metadata_export_is_cut_and_does_not_stop_the_others() {
    let (_tmp, dir) = staged_with_good();
    add_probe(&dir, "spinner", "id=spinner\nmeta=spin\n");
    let started = Instant::now();
    let found = discover(&dir).await;
    eprintln!("spinning metadata: discovery took {:?}", started.elapsed());
    let metas = found.unwrap_or_else(|e| panic!("the spinner must not fail discovery of the others: {e}"));
    assert_eq!(ids(&metas), ["good"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_metadata_export_does_not_stop_the_others() {
    let (_tmp, dir) = staged_with_good();
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
    let metas = tokio::time::timeout(DISCOVERY_BOUND, loader.discover(&plugins_dir))
        .await
        .map_err(|_| "discover hung".to_owned())?
        .map_err(|e| format!("discover: {e}"))?;
    let id = metas
        .iter()
        .map(|m| m.id.clone())
        .next()
        .ok_or("no metadata discovered")?;
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

#[tokio::test(flavor = "multi_thread")]
async fn a_dot_dot_plugin_id_cannot_write_outside_plugins_dir() {
    let outcome = load_probe_with_id("id=../escape\n").await;
    match outcome {
        Ok((tmp, plugins_dir)) => {
            let leaked = escapes(tmp.path(), &plugins_dir);
            assert!(
                leaked.is_empty(),
                "a `..` in the plugin id let the guest write outside plugins_dir: {leaked:?}"
            );
        }
        Err(reason) => {
            assert!(
                reason.contains("id") || reason.contains("load") || reason.contains("format"),
                "a `..` id should be refused with a clear error, got: {reason}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_absolute_plugin_id_cannot_redirect_the_data_dir() {
    let target = tempfile::tempdir().unwrap();
    let id = format!("id={}\n", target.path().join("stolen").display());
    let outcome = load_probe_with_id(&id).await;
    if outcome.is_ok() {
        let leaked = escapes(target.path(), Path::new("/nonexistent-plugins"));
        assert!(
            leaked.is_empty(),
            "an absolute plugin id redirected the data dir outside plugins_dir: {leaked:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_plugin_id_is_handled_without_a_panic() {
    let outcome = load_probe_with_id("id=\n").await;
    eprintln!("empty id outcome: {outcome:?}");
    if let Ok((tmp, plugins_dir)) = outcome {
        assert!(escapes(tmp.path(), &plugins_dir).is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_unicode_plugin_id_loads_into_a_scoped_data_dir() {
    let (_tmp, plugins_dir) = load_probe_with_id("id=café-plugin\n").await.expect("unicode id loads");
    assert!(
        plugins_dir.join("café-plugin").join("log.txt").exists(),
        "the data dir is scoped under the id"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_very_long_plugin_id_does_not_crash_the_loader() {
    let id = "a".repeat(300);
    let outcome = load_probe_with_id(&format!("id={id}\n")).await;
    eprintln!("300-char id outcome ok={}", outcome.is_ok());
}
