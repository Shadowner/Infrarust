//! [`PluginLoader`] trait, [`PluginContextFactory`] trait, and [`LoaderError`]
//! for plugin discovery and loading.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::event::BoxFuture;
use crate::plugin::{Plugin, PluginContext, PluginMetadata};

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

/// Passed to loaders during `load()` so they can bind proxy services
/// to the plugin runtime (e.g., WASM host functions).
pub trait PluginContextFactory: Send + Sync {
    fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext>;
}

/// Discovers and loads plugins from a specific source format.
///
/// The lifecycle is:
/// 1. `discover(plugin_dir)` — returns metadata
/// 2. `PluginManager` resolves dependencies (topological sort)
/// 3. `on_load(context_factory)` — once per loader, before any plugin loads
/// 4. `load(plugin_id, context_factory)` — builds the plugin
/// 5. `PluginManager` calls `on_enable(ctx)`
/// 6. At shutdown: `unload(plugin_id)` after `on_disable()`
/// 7. `on_shutdown()` — once per loader, after all its plugins are unloaded
///
/// Note that `discover()` runs in step 1, *before* the `PluginContextFactory`
/// (and thus the hosted runtime) exists: a loader must not depend on its own
/// runtime to enumerate plugins.
pub trait PluginLoader: Send + Sync {
    fn name(&self) -> &str;

    /// Explores plugin sources and returns metadata for all loadable plugins.
    fn discover<'a>(
        &'a self,
        plugin_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Vec<PluginMetadata>, LoaderError>>;

    /// Initializes the loader's runtime once, before any of its plugins load.
    ///
    /// Runs after [`discover`](Self::discover). On `Err`, the loader must roll
    /// back any partially-initialized state: [`on_shutdown`](Self::on_shutdown)
    /// is only called for loaders whose `on_load` returned `Ok`. When `on_load`
    /// fails, all of that loader's plugins are skipped and marked errored.
    ///
    /// The default implementation is a no-op.
    fn on_load<'a>(
        &'a self,
        context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<(), LoaderError>> {
        let _ = context_factory;
        Box::pin(async { Ok(()) })
    }

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
    fn unload<'a>(&'a self, plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>>;

    /// Tears down the loader's runtime at proxy shutdown, after every one of its
    /// plugins has been disabled and unloaded.
    ///
    /// Only called for loaders whose [`on_load`](Self::on_load) returned `Ok`.
    /// Must be idempotent. The default implementation is a no-op.
    fn on_shutdown<'a>(&'a self) -> BoxFuture<'a, Result<(), LoaderError>> {
        Box::pin(async { Ok(()) })
    }
}
