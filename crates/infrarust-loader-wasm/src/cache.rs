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

use crate::consts::{WASMTIME_CACHE_TAG, WORLD_VERSION};
use crate::error::WasmLoaderError;

#[derive(Clone)]
pub(crate) struct AotCache {
    dir: PathBuf,
}

impl AotCache {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn cache_key(wasm: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(wasm);
        hasher.update(b"\0");
        hasher.update(WASMTIME_CACHE_TAG.as_bytes());
        hasher.update(b"\0");
        hasher.update(WORLD_VERSION.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub(crate) fn compile_or_load(
        &self,
        engine: &Engine,
        wasm_path: &Path,
    ) -> Result<Component, WasmLoaderError> {
        let bytes = std::fs::read(wasm_path).map_err(|source| WasmLoaderError::CacheIo {
            path: wasm_path.to_path_buf(),
            source,
        })?;
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
                    reason: e.to_string(),
                })?;
        self.atomic_write(&cwasm, &serialized)?;

        deserialize_trusted(engine, &cwasm).map_err(|e| WasmLoaderError::Deserialize {
            path: cwasm,
            reason: e.to_string(),
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

/// SAFETY: only ever called on artifacts written by `AotCache::compile_or_load` into our
/// own cache dir (see module docs). We never deserialize a user-supplied `.cwasm`, and the
/// caller has confirmed via `detect_precompiled_file` that the file is a component.
#[allow(unsafe_code)]
fn deserialize_trusted(engine: &Engine, path: &Path) -> Result<Component, wasmtime::Error> {
    unsafe { Component::deserialize_file(engine, path) }
}
