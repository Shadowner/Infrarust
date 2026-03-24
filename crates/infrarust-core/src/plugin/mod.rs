//! Plugin context and lifecycle management.

pub mod context;
pub mod context_factory;
pub mod dependency;
pub mod loader;
pub mod manager;
pub mod plugin_registry_impl;
pub mod static_loader;
pub mod tracking;

pub use context_factory::{PluginContextFactory, PluginContextFactoryImpl, PluginPermissions};
pub use plugin_registry_impl::PluginRegistryImpl;
pub use loader::{LoaderError, PluginLoader};
pub use static_loader::{PluginFactory, StaticPluginLoader};

/// Tracks the lifecycle state of a plugin.
#[derive(Debug, Clone)]
pub enum PluginState {
    /// The plugin is being loaded (`on_enable` in progress).
    Loading,
    /// The plugin is active.
    Enabled,
    /// The plugin has been disabled.
    Disabled,
    /// The plugin encountered an error during initialization.
    Error(String),
}
