//! [`PluginLoader`] trait and [`LoaderError`] for plugin discovery and loading.

use std::path::{Path, PathBuf};

use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginMetadata};

use super::context_factory::PluginContextFactory;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoaderError {
    #[error("plugin directory not accessible: {path}")]
    DirectoryNotAccessible {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("plugin not found: {plugin_id}")]
    PluginNotFound { plugin_id: String },

    #[error("invalid plugin format at {path}: {reason}")]
    InvalidFormat { path: PathBuf, reason: String },

    #[error("failed to load plugin '{plugin_id}': {reason}")]
    LoadFailed {
        plugin_id: String,
        reason: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("failed to unload plugin '{plugin_id}': {reason}")]
    UnloadFailed { plugin_id: String, reason: String },

    #[error("duplicate plugin id '{plugin_id}' (found in loader '{first}' and '{second}')")]
    DuplicateId {
        plugin_id: String,
        first: String,
        second: String,
    },
}

/// Discovers and loads plugins from a specific source format.
///
/// The lifecycle is:
/// 1. `discover(plugin_dir)` — returns metadata
/// 2. `PluginManager` resolves dependencies (topological sort)
/// 3. `load(plugin_id, context_factory)` — builds the plugin
/// 4. `PluginManager` calls `on_enable(ctx)`
/// 5. At shutdown: `unload(plugin_id)` after `on_disable()`
pub trait PluginLoader: Send + Sync {
    fn name(&self) -> &str;

    /// Explores plugin sources and returns metadata for all loadable plugins.
    fn discover<'a>(
        &'a self,
        plugin_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Vec<PluginMetadata>, LoaderError>>;

    /// Loads a previously discovered plugin by its ID.
    ///
    /// The `context_factory` allows the loader to bind proxy services
    /// to the plugin runtime (e.g., WASM host functions).
    fn load<'a>(
        &'a self,
        plugin_id: &'a str,
        context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>>;

    /// Loader-specific cleanup after a plugin is disabled.
    fn unload<'a>(
        &'a self,
        plugin_id: &'a str,
    ) -> BoxFuture<'a, Result<(), LoaderError>>;
}
