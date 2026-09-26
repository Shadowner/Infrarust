//! Plugin context and lifecycle management.

pub mod context;
pub mod context_factory;
pub mod dependency;
pub mod loader;
pub mod manager;
pub mod plugin_registry_impl;
pub mod service_registry;
pub mod static_loader;
pub mod tracking;

pub use context_factory::{PluginContextFactory, PluginContextFactoryImpl, PluginPermissions};
pub use infrarust_api::plugin::PluginState;
pub use loader::{LoaderError, PluginLoader};
pub use plugin_registry_impl::PluginRegistryImpl;
pub use static_loader::{PluginFactory, StaticPluginLoader};
