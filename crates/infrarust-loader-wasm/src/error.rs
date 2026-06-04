use std::path::PathBuf;

use infrarust_api::error::PluginError;
use infrarust_api::loader::LoaderError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WasmLoaderError {
    /// Failed to build or configure the wasmtime engine.
    #[error("wasmtime engine error: {0}")]
    Engine(wasmtime::Error),

    /// AOT precompilation of a component failed.
    #[error("failed to precompile component at {path}: {reason}")]
    Precompile { path: PathBuf, reason: String },

    /// Deserializing a cached `.cwasm` artifact failed (version mismatch / corruption).
    #[error("failed to deserialize cached artifact at {path}: {reason}")]
    Deserialize { path: PathBuf, reason: String },

    /// Filesystem error while reading plugins or maintaining the AOT cache.
    #[error("cache io error at {path}: {source}")]
    CacheIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to set up the per-plugin WASI context (e.g. preopen of `data_dir`).
    #[error("wasi setup failed for {path}: {source}")]
    WasiSetup {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Linking or instantiating a component failed (missing import, type mismatch, trap).
    #[error("failed to instantiate plugin '{plugin_id}': {reason}")]
    Instantiate { plugin_id: String, reason: String },

    /// A guest export trapped (panic/OOB/epoch interrupt/resource limit) during a call.
    #[error("wasm guest '{plugin_id}' trapped during {op}: {reason}")]
    Trap {
        plugin_id: String,
        op: &'static str,
        reason: String,
    },

    /// A guest export returned `Err(string)` from a `result<_, string>` (not a trap).
    #[error("wasm guest '{plugin_id}' returned an error from {op}: {message}")]
    GuestError {
        plugin_id: String,
        op: &'static str,
        message: String,
    },

    /// Could not extract or validate [`PluginMetadata`] from a component.
    ///
    /// [`PluginMetadata`]: infrarust_api::plugin::PluginMetadata
    #[error("failed to extract metadata from {path}: {reason}")]
    Metadata { path: PathBuf, reason: String },

    /// The component targets an incompatible world / major version.
    #[error("plugin at {path} targets incompatible world '{found}', expected '{expected}'")]
    WorldIncompatible {
        path: PathBuf,
        expected: String,
        found: String,
    },

    #[error("plugin '{plugin_id}' imports a host interface it lacks the capability for: {reason}")]
    CapabilityDenied { plugin_id: String, reason: String },
}

impl WasmLoaderError {
    pub(crate) fn into_plugin_error(self) -> PluginError {
        PluginError::InitFailed(self.to_string())
    }

    pub(crate) fn into_loader_error(self, plugin_id: &str) -> LoaderError {
        match self {
            WasmLoaderError::CacheIo { path, source }
            | WasmLoaderError::WasiSetup { path, source } => {
                LoaderError::DirectoryNotAccessible { path, source }
            }
            WasmLoaderError::Metadata { path, reason } => {
                LoaderError::InvalidFormat { path, reason }
            }
            WasmLoaderError::WorldIncompatible {
                path,
                found,
                expected,
            } => LoaderError::InvalidFormat {
                path,
                reason: format!("incompatible world '{found}', expected '{expected}'"),
            },
            other => LoaderError::LoadFailed {
                plugin_id: plugin_id.to_owned(),
                reason: other.to_string(),
                source: None,
            },
        }
    }
}
