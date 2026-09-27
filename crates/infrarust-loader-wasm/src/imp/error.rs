use std::path::PathBuf;
use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::loader::LoaderError;

use crate::consts::ERROR_TEXT_LIMIT;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WasmLoaderError {
    /// Failed to build or configure the wasmtime engine.
    #[error("wasmtime engine error: {0}")]
    Engine(wasmtime::Error),

    #[error("invalid wasm configuration: {0}")]
    Config(String),

    #[error("cannot read the plugin file: {source}")]
    ReadPlugin {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("not a WebAssembly component: {reason}")]
    NotAComponent { path: PathBuf, reason: String },

    #[error("the plugin file is {len} bytes, above the {limit}-byte limit for a component")]
    TooLarge { path: PathBuf, len: u64, limit: u64 },

    /// AOT precompilation of a component failed.
    #[error("failed to precompile component at {path}: {reason}")]
    Precompile { path: PathBuf, reason: String },

    #[error("failed to load the compiled component of {path}: {reason}")]
    Deserialize { path: PathBuf, reason: String },

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
    #[error("wasm guest '{plugin_id}' trapped during {op}: {trap:#}")]
    Trap {
        plugin_id: String,
        op: &'static str,
        trap: Arc<wasmtime::Error>,
    },

    #[error("wasm guest '{plugin_id}' could not run {op}: {source}")]
    CallFailed {
        plugin_id: String,
        op: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Could not extract or validate [`PluginMetadata`] from a component.
    ///
    /// [`PluginMetadata`]: infrarust_api::plugin::PluginMetadata
    #[error("failed to extract metadata from {path}: {reason}")]
    Metadata { path: PathBuf, reason: String },

    /// The component targets an incompatible world / major version.
    #[error(
        "plugin built for {found}; this host supports {expected}, rebuild it with an infrarust-plugin-sdk that targets {expected}"
    )]
    WorldIncompatible {
        path: PathBuf,
        expected: String,
        found: String,
    },

    #[error("not an Infrarust plugin component: it exports no infrarust:plugin/guest interface")]
    NotAPlugin { path: PathBuf },

    #[error("plugin '{plugin_id}' imports a host interface it lacks the capability for: {reason}")]
    CapabilityDenied { plugin_id: String, reason: String },
}

impl WasmLoaderError {
    pub(crate) fn into_plugin_error(self) -> PluginError {
        match self {
            Self::Instantiate { .. } | Self::CapabilityDenied { .. } => {
                PluginError::InitFailed(self.to_string())
            }
            Self::Engine(_)
            | Self::Config(_)
            | Self::ReadPlugin { .. }
            | Self::NotAComponent { .. }
            | Self::TooLarge { .. }
            | Self::Precompile { .. }
            | Self::Deserialize { .. }
            | Self::WasiSetup { .. }
            | Self::Trap { .. }
            | Self::CallFailed { .. }
            | Self::Metadata { .. }
            | Self::WorldIncompatible { .. }
            | Self::NotAPlugin { .. } => PluginError::Other(Box::new(self)),
        }
    }

    pub(crate) fn into_loader_error(self, plugin_id: &str) -> LoaderError {
        match self {
            WasmLoaderError::WasiSetup { path, source } => {
                LoaderError::DirectoryNotAccessible { path, source }
            }
            WasmLoaderError::Metadata { path, reason } => LoaderError::InvalidFormat {
                path,
                reason: bounded(reason),
            },
            WasmLoaderError::NotAComponent { ref path, .. }
            | WasmLoaderError::TooLarge { ref path, .. } => LoaderError::InvalidFormat {
                path: path.clone(),
                reason: bounded(&self),
            },
            WasmLoaderError::WorldIncompatible {
                path,
                found,
                expected,
            } => LoaderError::InvalidFormat {
                path,
                reason: format!(
                    "plugin built for {found}; this host supports {expected}, rebuild it with an infrarust-plugin-sdk that targets {expected}"
                ),
            },
            WasmLoaderError::NotAPlugin { path } => LoaderError::InvalidFormat {
                path,
                reason: "not an Infrarust plugin component".to_owned(),
            },
            other => LoaderError::LoadFailed {
                plugin_id: plugin_id.to_owned(),
                reason: bounded(&other),
                source: Some(Box::new(other)),
            },
        }
    }
}

pub(crate) fn bounded(value: impl std::fmt::Display) -> String {
    use std::fmt::Write;
    let mut text = BoundedText::default();
    let _ = write!(text, "{value}");
    if text.cut {
        text.kept.push_str(" [...]");
    }
    text.kept
}

#[derive(Default)]
struct BoundedText {
    kept: String,
    cut: bool,
}

impl std::fmt::Write for BoundedText {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let room = ERROR_TEXT_LIMIT - self.kept.len();
        if s.len() <= room {
            self.kept.push_str(s);
            return Ok(());
        }
        let mut end = room;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.kept.push_str(&s[..end]);
        self.cut = true;
        Err(std::fmt::Error)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn a_short_text_is_kept_whole() {
        assert_eq!(bounded("short reason"), "short reason");
    }

    #[test]
    fn a_long_text_is_cut_at_the_limit_on_a_char_boundary() {
        let long = "é".repeat(ERROR_TEXT_LIMIT);
        let text = bounded(&long);
        assert!(
            text.len() <= ERROR_TEXT_LIMIT + " [...]".len(),
            "{}",
            text.len()
        );
        assert!(text.ends_with(" [...]"));
        assert!(text.trim_end_matches(" [...]").chars().all(|c| c == 'é'));
    }

    #[test]
    fn formatting_stops_once_the_limit_is_reached() {
        struct Endless;
        impl std::fmt::Display for Endless {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                loop {
                    f.write_str("chunk ")?;
                }
            }
        }
        assert!(bounded(Endless).len() <= ERROR_TEXT_LIMIT + " [...]".len());
    }
}
