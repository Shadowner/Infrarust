//! Plugin lifecycle traits and metadata.
//!
//! The [`Plugin`] trait is the entry point for all Infrarust plugins.
//! Plugins register event listeners, commands, and handlers during
//! [`on_enable`](Plugin::on_enable) via the [`PluginContext`].

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::command::CommandManager;
use crate::error::PluginError;
use crate::event::BoxFuture;
use crate::event::bus::EventBus;
use crate::filter::registry::{CodecFilterRegistry, TransportFilterRegistry};
use crate::limbo::LimboHandler;
use crate::services::{
    ban_service::BanService, config_service::ConfigService, player_registry::PlayerRegistry,
    plugin_registry::PluginRegistry, proxy_info::ProxyInfo, scheduler::Scheduler,
    server_manager::ServerManager,
};

/// Metadata describing a plugin.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginMetadata {
    /// Unique `snake_case` identifier (e.g. `"my_plugin"`).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semver version string.
    pub version: String,
    /// Plugin authors.
    pub authors: Vec<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Other plugins this plugin depends on.
    pub dependencies: Vec<PluginDependency>,
}

/// A dependency on another plugin.
#[derive(Debug, Clone)]
pub struct PluginDependency {
    /// The ID of the required plugin.
    pub id: String,
    /// If `true`, the plugin can function without this dependency.
    pub optional: bool,
}

/// The main trait that all Infrarust plugins implement.
///
/// # Example
/// ```ignore
/// use infrarust_api::prelude::*;
///
/// struct MyPlugin;
///
/// impl Plugin for MyPlugin {
///     fn metadata(&self) -> PluginMetadata {
///         PluginMetadata::new("my_plugin", "My Plugin", "1.0.0")
///             .author("Author")
///             .description("A cool plugin")
///     }
///
///     fn on_enable<'a>(&'a self, ctx: &'a dyn PluginContext) -> BoxFuture<'a, Result<(), PluginError>> {
///         Box::pin(async move {
///             // Register event listeners, commands, etc.
///             Ok(())
///         })
///     }
/// }
/// ```
pub trait Plugin: Send + Sync {
    fn metadata(&self) -> PluginMetadata;

    /// Called when the plugin is enabled (proxy startup or hot-load).
    ///
    /// Use the [`PluginContext`] to register event listeners, commands,
    /// limbo handlers, and access proxy services.
    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>>;

    /// Called when the plugin is disabled (proxy shutdown or hot-unload).
    ///
    /// Override this to clean up resources. The default implementation
    /// does nothing.
    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async { Ok(()) })
    }
}

impl PluginMetadata {
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            authors: vec![],
            description: None,
            dependencies: vec![],
        }
    }

    /// Adds an author.
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.authors.push(author.into());
        self
    }

    /// Sets the description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Adds a required dependency.
    pub fn depends_on(mut self, id: impl Into<String>) -> Self {
        self.dependencies.push(PluginDependency {
            id: id.into(),
            optional: false,
        });
        self
    }

    /// Adds an optional dependency.
    pub fn optional_dependency(mut self, id: impl Into<String>) -> Self {
        self.dependencies.push(PluginDependency {
            id: id.into(),
            optional: true,
        });
        self
    }
}

pub mod private {
    /// Sealed — only the proxy implements [`PluginContext`](super::PluginContext).
    pub trait Sealed {}
}

/// Context provided to plugins during [`Plugin::on_enable`].
///
/// Gives access to all proxy services and registration methods.
/// The proxy is the sole implementor.
pub trait PluginContext: Send + Sync + private::Sealed {
    /// Used internally by the plugin manager for cleanup via downcast.
    fn as_any(&self) -> &dyn std::any::Any;

    fn event_bus(&self) -> &dyn EventBus;

    fn player_registry(&self) -> &dyn PlayerRegistry;

    /// Returns an `Arc` handle to the player registry, suitable for
    /// capturing in closures and event handlers.
    fn player_registry_handle(&self) -> Arc<dyn PlayerRegistry>;

    fn server_manager(&self) -> &dyn ServerManager;

    fn server_manager_handle(&self) -> Arc<dyn ServerManager>;

    fn ban_service(&self) -> &dyn BanService;

    fn ban_service_handle(&self) -> Arc<dyn BanService>;

    fn config_service(&self) -> &dyn ConfigService;

    fn config_service_handle(&self) -> Arc<dyn ConfigService>;

    fn command_manager(&self) -> &dyn CommandManager;

    fn scheduler(&self) -> &dyn Scheduler;

    fn event_bus_handle(&self) -> Arc<dyn EventBus>;

    /// Registers a limbo handler for this plugin.
    ///
    /// The handler's [`name()`](LimboHandler::name) must match the name
    /// referenced in server configuration `limbo_handlers` lists.
    fn register_limbo_handler(&self, handler: Box<dyn LimboHandler>);

    /// Returns the codec filter registry for registering packet-level filters.
    ///
    /// Returns `Some` for native plugins, `None` for WASM plugins (future).
    fn codec_filters(&self) -> Option<&dyn CodecFilterRegistry>;

    /// Returns the transport filter registry for registering TCP-level filters.
    ///
    /// Returns `Some` for native plugins, `None` for WASM plugins.
    fn transport_filters(&self) -> Option<&dyn TransportFilterRegistry>;

    fn plugin_registry(&self) -> &dyn PluginRegistry;

    fn plugin_registry_handle(&self) -> Arc<dyn PluginRegistry>;

    fn register_config_provider(&self, provider: Box<dyn crate::provider::PluginConfigProvider>);

    fn plugin_id(&self) -> &str;

    fn data_dir(&self) -> PathBuf;

    fn proxy_shutdown(&self) -> CancellationToken;

    fn proxy_info(&self) -> &ProxyInfo;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn test_metadata_builder_minimal() {
        let meta = PluginMetadata::new("test", "Test Plugin", "1.0.0");
        assert_eq!(meta.id, "test");
        assert_eq!(meta.name, "Test Plugin");
        assert_eq!(meta.version, "1.0.0");
        assert!(meta.authors.is_empty());
        assert!(meta.description.is_none());
        assert!(meta.dependencies.is_empty());
    }

    #[test]
    fn test_metadata_builder_full() {
        let meta = PluginMetadata::new("my_plugin", "My Plugin", "2.0.0")
            .author("Alice")
            .author("Bob")
            .description("A great plugin")
            .depends_on("core_plugin")
            .optional_dependency("extra_plugin");

        assert_eq!(meta.id, "my_plugin");
        assert_eq!(meta.authors, vec!["Alice", "Bob"]);
        assert_eq!(meta.description.as_deref(), Some("A great plugin"));
        assert_eq!(meta.dependencies.len(), 2);
        assert_eq!(meta.dependencies[0].id, "core_plugin");
        assert!(!meta.dependencies[0].optional);
        assert_eq!(meta.dependencies[1].id, "extra_plugin");
        assert!(meta.dependencies[1].optional);
    }
}
