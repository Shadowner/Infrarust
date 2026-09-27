use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use wasmtime::component::Component;
use wasmtime::{Engine, Precompiled};

use crate::consts::{MAX_COMPONENT_BYTES, WORLD_VERSION};
use crate::error::{WasmLoaderError, bounded};

const WASM_MAGIC: &[u8; 4] = b"\0asm";
const COMPONENT_HEADER: [u8; 4] = [0x0d, 0x00, 0x01, 0x00];
const CORE_MODULE_HEADER: [u8; 4] = [0x01, 0x00, 0x00, 0x00];
const PREAMBLE_LEN: usize = 8;
const ENTRY_SUFFIX: &str = ".cwasm";
const TEMP_SUFFIX: &str = ".cwasm.tmp";
const TEMP_RANDOM_CHARS: usize = 12;
const ORPHANED_TEMP_AGE: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub(crate) struct AotCache {
    dir: PathBuf,
    engine_tag: String,
    write_warned: Arc<AtomicBool>,
}

impl AotCache {
    pub(crate) fn new(engine: &Engine, dir: PathBuf) -> Self {
        Self {
            dir,
            engine_tag: engine_tag(engine),
            write_warned: Arc::new(AtomicBool::new(false)),
        }
    }

    fn cache_key(&self, wasm: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(wasm);
        hasher.update(b"\0");
        hasher.update(self.engine_tag.as_bytes());
        hasher.update(b"\0");
        hasher.update(WORLD_VERSION.as_bytes());
        hex(&hasher.finalize())
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}{ENTRY_SUFFIX}"))
    }

    pub(crate) fn read_source(&self, wasm_path: &Path) -> Result<Source, WasmLoaderError> {
        let bytes = read_component(wasm_path)?;
        let key = self.cache_key(&bytes);
        Ok(Source { bytes, key })
    }

    pub(crate) fn compile_or_load(
        &self,
        engine: &Engine,
        wasm_path: &Path,
        source: &Source,
    ) -> Result<Component, WasmLoaderError> {
        let Source { bytes, key } = source;
        let entry = self.entry_path(key);

        let stale = match self.read_entry(&entry) {
            Some(artifact) => match load_artifact(engine, &artifact) {
                Ok(component) => return Ok(component),
                Err(error) => Some(error),
            },
            None => None,
        };

        let artifact =
            engine
                .precompile_component(bytes)
                .map_err(|e| WasmLoaderError::Precompile {
                    path: wasm_path.to_path_buf(),
                    reason: bounded(format_args!("{e:#}")),
                })?;
        let loaded = load_artifact(engine, &artifact);
        match (&stale, &loaded) {
            (Some(_), Err(_)) => {}
            (Some(error), Ok(_)) => {
                tracing::warn!(
                    path = %entry.display(),
                    error = %bounded(format_args!("{error:#}")),
                    "AOT cache entry could not be loaded; replaced by a fresh compilation"
                );
                self.store(key, &artifact);
            }
            (None, _) => self.store(key, &artifact),
        }
        loaded.map_err(|e| WasmLoaderError::Deserialize {
            path: wasm_path.to_path_buf(),
            reason: bounded(format_args!("{e:#}")),
        })
    }

    pub(crate) fn sweep(&self, live: &HashSet<String>) {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                tracing::debug!(
                    cache_dir = %self.dir.display(),
                    error = %error,
                    "AOT cache directory cannot be listed; nothing removed"
                );
                return;
            }
        };
        let now = SystemTime::now();
        let mut removed = 0usize;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let unused = match name.to_str().and_then(CacheFile::parse) {
                Some(CacheFile::Entry(key)) => !live.contains(key),
                Some(CacheFile::Temp) => is_orphaned(&entry, now),
                None => false,
            };
            if !unused {
                continue;
            }
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::debug!(
                    path = %entry.path().display(),
                    error = %error,
                    "unused AOT cache file cannot be removed"
                ),
            }
        }
        if removed > 0 {
            tracing::debug!(
                cache_dir = %self.dir.display(),
                removed,
                "AOT cache files no discovered plugin uses were removed"
            );
        }
    }

    fn read_entry(&self, entry: &Path) -> Option<Vec<u8>> {
        match std::fs::symlink_metadata(entry) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => {
                tracing::warn!(
                    path = %entry.display(),
                    "AOT cache entry is not a regular file; ignored"
                );
                return None;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(
                    path = %entry.display(),
                    error = %error,
                    "AOT cache entry cannot be read; ignored"
                );
                return None;
            }
        }
        match std::fs::read(entry) {
            Ok(artifact) => Some(artifact),
            Err(error) => {
                tracing::warn!(
                    path = %entry.display(),
                    error = %error,
                    "AOT cache entry cannot be read; ignored"
                );
                None
            }
        }
    }

    fn store(&self, key: &str, artifact: &[u8]) {
        if let Err(error) = self.write_entry(key, artifact)
            && !self.write_warned.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(
                cache_dir = %self.dir.display(),
                error = %error,
                "AOT cache directory cannot be written; plugins are compiled in memory and compiled again at the next start"
            );
        }
    }

    fn write_entry(&self, key: &str, artifact: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        create_private_dir(&self.dir)?;
        let mut temp = tempfile::Builder::new()
            .prefix(&format!("{key}."))
            .suffix(TEMP_SUFFIX)
            .rand_bytes(TEMP_RANDOM_CHARS)
            .tempfile_in(&self.dir)?;
        temp.write_all(artifact)?;
        temp.as_file().sync_all()?;
        temp.persist(self.entry_path(key))
            .map(drop)
            .map_err(|error| error.error)
    }
}

pub(crate) struct Source {
    bytes: Vec<u8>,
    key: String,
}

impl Source {
    pub(crate) fn key(&self) -> &str {
        &self.key
    }
}

enum CacheFile<'a> {
    Entry(&'a str),
    Temp,
}

impl<'a> CacheFile<'a> {
    fn parse(name: &'a str) -> Option<Self> {
        if let Some(stem) = name.strip_suffix(TEMP_SUFFIX) {
            let (key, random) = stem.split_once('.')?;
            let random_ok = random.len() == TEMP_RANDOM_CHARS
                && random.bytes().all(|b| b.is_ascii_alphanumeric());
            return (is_key(key) && random_ok).then_some(Self::Temp);
        }
        let key = name.strip_suffix(ENTRY_SUFFIX)?;
        is_key(key).then_some(Self::Entry(key))
    }
}

fn is_key(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_orphaned(entry: &std::fs::DirEntry, now: SystemTime) -> bool {
    entry
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= ORPHANED_TEMP_AGE)
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn load_artifact(engine: &Engine, artifact: &[u8]) -> Result<Component, wasmtime::Error> {
    if Engine::detect_precompiled(artifact) != Some(Precompiled::Component) {
        return Err(wasmtime::Error::msg(
            "not a precompiled component of this engine",
        ));
    }
    deserialize(engine, artifact)
}

#[allow(unsafe_code)]
fn deserialize(engine: &Engine, artifact: &[u8]) -> Result<Component, wasmtime::Error> {
    unsafe { Component::deserialize(engine, artifact) }
}

fn engine_tag(engine: &Engine) -> String {
    use std::hash::Hash;
    let mut hasher = DigestHasher(Sha256::new());
    engine.precompile_compatibility_hash().hash(&mut hasher);
    hex(&hasher.0.finalize())
}

struct DigestHasher(Sha256);

impl std::hash::Hasher for DigestHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        let digest = self.0.clone().finalize();
        let mut first = [0u8; 8];
        first.copy_from_slice(&digest[..8]);
        u64::from_le_bytes(first)
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, b| {
            let _ = write!(text, "{b:02x}");
            text
        })
}

pub(crate) fn read_component(path: &Path) -> Result<Vec<u8>, WasmLoaderError> {
    use std::io::Read;
    let unreadable = |source| WasmLoaderError::ReadPlugin {
        path: path.to_path_buf(),
        source,
    };
    let mut file = std::fs::File::open(path).map_err(unreadable)?;
    let mut preamble = [0u8; PREAMBLE_LEN];
    let filled = read_preamble(&mut file, &mut preamble).map_err(unreadable)?;
    check_preamble(&preamble[..filled]).map_err(|reason| WasmLoaderError::NotAComponent {
        path: path.to_path_buf(),
        reason,
    })?;
    let too_large = |len| WasmLoaderError::TooLarge {
        path: path.to_path_buf(),
        len,
        limit: MAX_COMPONENT_BYTES,
    };
    let len = file.metadata().map_err(unreadable)?.len();
    if len > MAX_COMPONENT_BYTES {
        return Err(too_large(len));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or_default());
    bytes.extend_from_slice(&preamble);
    let rest = MAX_COMPONENT_BYTES + 1 - PREAMBLE_LEN as u64;
    file.take(rest)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    let read = bytes.len() as u64;
    if read > MAX_COMPONENT_BYTES {
        return Err(too_large(read));
    }
    Ok(bytes)
}

fn read_preamble(file: &mut std::fs::File, preamble: &mut [u8]) -> std::io::Result<usize> {
    use std::io::Read;
    let mut filled = 0;
    while filled < preamble.len() {
        match file.read(&mut preamble[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn check_preamble(preamble: &[u8]) -> Result<(), String> {
    if preamble.is_empty() {
        return Err("the file is empty".to_owned());
    }
    if preamble.len() < PREAMBLE_LEN && WASM_MAGIC.starts_with(&preamble[..preamble.len().min(4)]) {
        return Err("the file ends inside the WebAssembly header".to_owned());
    }
    if !preamble.starts_with(WASM_MAGIC) {
        return Err(if looks_like_text(preamble) {
            "WebAssembly text format is not accepted, compile it to a binary component".to_owned()
        } else {
            "the file does not start with the WebAssembly magic bytes \\0asm".to_owned()
        });
    }
    match [preamble[4], preamble[5], preamble[6], preamble[7]] {
        COMPONENT_HEADER => Ok(()),
        CORE_MODULE_HEADER => Err(
            "it is a core WebAssembly module; build the plugin as a component for wasm32-wasip2 with the Infrarust plugin SDK"
                .to_owned(),
        ),
        [low, high, 0x01, 0x00] => Err(format!(
            "it is a component in binary encoding version {}, this host reads version {}",
            u16::from_le_bytes([low, high]),
            u16::from_le_bytes([COMPONENT_HEADER[0], COMPONENT_HEADER[1]])
        )),
        _ => Err("its WebAssembly header names an unknown version or layer".to_owned()),
    }
}

fn looks_like_text(preamble: &[u8]) -> bool {
    preamble
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|first| matches!(first, b'(' | b';'))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn refusal(bytes: &[u8]) -> String {
        check_preamble(bytes).expect_err("refused")
    }

    fn engine_with(settings: impl FnOnce(&mut wasmtime::Config)) -> Engine {
        let mut config = wasmtime::Config::new();
        config.epoch_interruption(true);
        config.wasm_component_model(true);
        config.consume_fuel(false);
        config.concurrency_support(false);
        settings(&mut config);
        Engine::new(&config).unwrap()
    }

    fn key_under(engine: &Engine) -> String {
        AotCache::new(engine, PathBuf::new()).cache_key(b"\0asm\x0d\0\x01\0")
    }

    fn released_with(version: &str) -> Engine {
        engine_with(|config| {
            config
                .module_version(wasmtime::ModuleVersionStrategy::Custom(version.to_owned()))
                .unwrap();
        })
    }

    fn wasmtime_version_in_the_lockfile() -> Option<String> {
        let lock =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
                .ok()?;
        let versions: Vec<String> = lock
            .split("[[package]]")
            .filter(|package| package.contains("\nname = \"wasmtime\"\n"))
            .filter_map(|package| {
                package
                    .lines()
                    .find_map(|line| line.strip_prefix("version = \""))
                    .map(|version| version.trim_end_matches('"').to_owned())
            })
            .collect();
        assert_eq!(
            versions.len(),
            1,
            "one wasmtime in Cargo.lock: {versions:?}"
        );
        versions.into_iter().next()
    }

    #[test]
    fn the_cache_key_follows_the_wasmtime_version_the_proxy_is_built_with() {
        let Some(version) = wasmtime_version_in_the_lockfile() else {
            return;
        };
        let built = key_under(&engine_with(|_| {}));
        assert_eq!(
            built,
            key_under(&released_with(&version)),
            "the key must be the one of wasmtime {version}, the version in Cargo.lock"
        );
        assert_ne!(built, key_under(&released_with("45.0.3")));
    }

    #[test]
    fn the_cache_key_changes_with_the_engine_settings_that_shape_the_code() {
        let base = key_under(&engine_with(|_| {}));
        assert_ne!(
            base,
            key_under(&engine_with(|config| {
                config.epoch_interruption(false);
            }))
        );
        assert_eq!(base, key_under(&engine_with(|_| {})));
    }

    #[test]
    fn the_cache_key_does_not_change_with_the_instance_pool() {
        let on_demand: infrarust_config::ProxyConfig = toml::from_str("").unwrap();
        let pooled: infrarust_config::ProxyConfig =
            toml::from_str("[wasm]\ninstance_pool = 8\n").unwrap();
        assert_eq!(
            key_under(&crate::engine::build_engine(&on_demand).unwrap()),
            key_under(&crate::engine::build_engine(&pooled).unwrap())
        );
    }

    #[test]
    fn the_sweep_removes_only_what_the_cache_wrote_and_nothing_uses() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = AotCache::new(&engine_with(|_| {}), tmp.path().to_path_buf());
        let used = "a".repeat(64);
        let unused = "b".repeat(64);
        let file = |name: &str| {
            let path = tmp.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            path
        };
        let aged = |path: &Path| {
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_modified(SystemTime::now() - ORPHANED_TEMP_AGE - Duration::from_secs(1))
                .unwrap();
        };
        let kept_entry = file(&format!("{used}.cwasm"));
        let dropped_entry = file(&format!("{unused}.cwasm"));
        let fresh_temp = file(&format!("{used}.abcdefABCDEF.cwasm.tmp"));
        let old_temp = file(&format!("{unused}.0123456789ab.cwasm.tmp"));
        aged(&old_temp);
        let foreign = [
            file("notes.txt"),
            file(&format!("{}.cwasm", "c".repeat(63))),
            file(&format!("{}.cwasm", "C".repeat(64))),
            file(&format!("{unused}.cwasm.bak")),
            file(&format!("{unused}.short.cwasm.tmp")),
        ];
        for path in &foreign {
            aged(path);
        }
        cache.sweep(&HashSet::from([used]));
        assert!(kept_entry.exists());
        assert!(!dropped_entry.exists());
        assert!(
            fresh_temp.exists(),
            "a temporary file being written is left alone"
        );
        assert!(!old_temp.exists());
        for path in &foreign {
            assert!(path.exists(), "{} is not the cache's", path.display());
        }
    }

    #[test]
    fn a_missing_cache_directory_is_not_created_by_the_sweep() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("absent");
        AotCache::new(&engine_with(|_| {}), dir.clone()).sweep(&HashSet::new());
        assert!(!dir.exists());
    }

    #[test]
    fn a_component_header_is_accepted() {
        assert!(check_preamble(b"\0asm\x0d\0\x01\0").is_ok());
    }

    #[test]
    fn each_kind_of_non_component_gets_its_own_reason() {
        assert_eq!(refusal(b""), "the file is empty");
        assert!(refusal(b"\0as").contains("ends inside the WebAssembly header"));
        assert!(refusal(b"\0asm\x0d\0").contains("ends inside the WebAssembly header"));
        assert!(refusal(b"\0asm\x01\0\0\0").contains("core WebAssembly module"));
        assert!(refusal(b"(compone").contains("text format is not accepted"));
        assert!(refusal(b"\n  ;; a c").contains("text format is not accepted"));
        assert!(refusal(b"\0\0\0\0\0\0\0\0").contains("magic bytes"));
        assert!(refusal(b"\x7fELF\x02\x01\x01\0").contains("magic bytes"));
        assert!(refusal(b"\0asm\x0e\0\x01\0").contains("encoding version 14"));
        assert!(refusal(b"\0asm\x01\0\x02\0").contains("unknown version or layer"));
    }

    #[test]
    fn a_file_above_the_size_limit_is_refused_before_it_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.wasm");
        let file = std::fs::File::create(&path).unwrap();
        std::io::Write::write_all(&mut &file, b"\0asm\x0d\0\x01\0").unwrap();
        file.set_len(MAX_COMPONENT_BYTES + 1).unwrap();
        drop(file);
        match read_component(&path) {
            Err(WasmLoaderError::TooLarge { len, limit, .. }) => {
                assert_eq!(len, MAX_COMPONENT_BYTES + 1);
                assert_eq!(limit, MAX_COMPONENT_BYTES);
            }
            other => panic!(
                "expected TooLarge, got {:?}",
                other.map(|bytes| bytes.len())
            ),
        }
    }

    #[test]
    fn a_component_file_is_read_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("small.wasm");
        let mut bytes = b"\0asm\x0d\0\x01\0".to_vec();
        bytes.extend(std::iter::repeat_n(7u8, 10_000));
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_component(&path).unwrap(), bytes);
    }
}
