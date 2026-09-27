//! Ahead-of-time compile cache (§10). Each `*.wasm` is precompiled to a `.cwasm` keyed by
//! its content hash plus the wasmtime/world version, so subsequent loads skip compilation.
//!
//! SAFETY model: `Component::deserialize*` is `unsafe`, and is invoked ONLY on artifacts
//! this loader produced via `precompile_component` and wrote atomically into its own cache
//! directory. Users deposit `*.wasm`; they never supply a `.cwasm`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use wasmtime::component::Component;
use wasmtime::{Engine, Precompiled};

use crate::consts::{MAX_COMPONENT_BYTES, WASMTIME_CACHE_TAG, WORLD_VERSION};
use crate::error::{WasmLoaderError, bounded};

const WASM_MAGIC: &[u8; 4] = b"\0asm";
const COMPONENT_HEADER: [u8; 4] = [0x0d, 0x00, 0x01, 0x00];
const CORE_MODULE_HEADER: [u8; 4] = [0x01, 0x00, 0x00, 0x00];
const PREAMBLE_LEN: usize = 8;

#[derive(Clone)]
pub(crate) struct AotCache {
    dir: PathBuf,
}

impl AotCache {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn cache_key(wasm: &[u8]) -> String {
        use std::fmt::Write;
        let mut hasher = Sha256::new();
        hasher.update(wasm);
        hasher.update(b"\0");
        hasher.update(WASMTIME_CACHE_TAG.as_bytes());
        hasher.update(b"\0");
        hasher.update(WORLD_VERSION.as_bytes());
        let digest = hasher.finalize();
        digest
            .iter()
            .fold(String::with_capacity(digest.len() * 2), |mut key, b| {
                let _ = write!(key, "{b:02x}");
                key
            })
    }

    pub(crate) fn compile_or_load(
        &self,
        engine: &Engine,
        wasm_path: &Path,
    ) -> Result<Component, WasmLoaderError> {
        let bytes = read_component(wasm_path)?;
        let key = Self::cache_key(&bytes);
        let cwasm = self.dir.join(format!("{key}.cwasm"));

        if cwasm.is_file() {
            match Engine::detect_precompiled_file(&cwasm) {
                Ok(Some(Precompiled::Component)) => match deserialize_trusted(engine, &cwasm) {
                    Ok(component) => return Ok(component),
                    Err(e) => {
                        tracing::warn!(path = %cwasm.display(), error = %e,
                            "stale or corrupt .cwasm, recompiling");
                        let _ = std::fs::remove_file(&cwasm);
                    }
                },
                _ => {
                    let _ = std::fs::remove_file(&cwasm);
                }
            }
        }

        let serialized =
            engine
                .precompile_component(&bytes)
                .map_err(|e| WasmLoaderError::Precompile {
                    path: wasm_path.to_path_buf(),
                    reason: bounded(format_args!("{e:#}")),
                })?;
        self.atomic_write(&cwasm, &serialized)?;

        deserialize_trusted(engine, &cwasm).map_err(|e| WasmLoaderError::Deserialize {
            path: cwasm,
            reason: bounded(format_args!("{e:#}")),
        })
    }

    fn atomic_write(&self, dst: &Path, data: &[u8]) -> Result<(), WasmLoaderError> {
        use std::io::Write;
        std::fs::create_dir_all(&self.dir).map_err(|source| WasmLoaderError::CacheIo {
            path: self.dir.clone(),
            source,
        })?;
        let tmp = dst.with_extension(format!("cwasm.tmp.{}", std::process::id()));
        let mut file = std::fs::File::create(&tmp).map_err(|source| WasmLoaderError::CacheIo {
            path: tmp.clone(),
            source,
        })?;
        file.write_all(data)
            .and_then(|()| file.sync_all())
            .map_err(|source| WasmLoaderError::CacheIo {
                path: tmp.clone(),
                source,
            })?;
        std::fs::rename(&tmp, dst).map_err(|source| WasmLoaderError::CacheIo {
            path: dst.to_path_buf(),
            source,
        })
    }
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

/// SAFETY: only ever called on artifacts written by `AotCache::compile_or_load` into our
/// own cache dir (see module docs). We never deserialize a user-supplied `.cwasm`, and the
/// caller has confirmed via `detect_precompiled_file` that the file is a component.
#[allow(unsafe_code)]
fn deserialize_trusted(engine: &Engine, path: &Path) -> Result<Component, wasmtime::Error> {
    unsafe { Component::deserialize_file(engine, path) }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn refusal(bytes: &[u8]) -> String {
        check_preamble(bytes).expect_err("refused")
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
