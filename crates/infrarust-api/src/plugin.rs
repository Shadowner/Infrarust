//! Plugin lifecycle traits and metadata.
//!
//! The [`Plugin`] trait is the entry point for all Infrarust plugins.
//! Plugins register event listeners, commands, and handlers during
//! [`on_enable`](Plugin::on_enable) via the [`PluginContext`].

use std::sync::Arc;

use crate::command::CommandManager;
use crate::error::PluginError;
use crate::event::bus::EventBus;
use crate::limbo::LimboHandler;
use crate::services::{
    ban_service::BanService, config_service::ConfigService, player_registry::PlayerRegistry,
    scheduler::Scheduler, server_manager::ServerManager,
};

/// Metadata describing a plugin.
#[derive(Debug, Clone)]
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
///         PluginMetadata {
///             id: "my_plugin".into(),
///             name: "My Plugin".into(),
///             version: "1.0.0".into(),
///             authors: vec!["Author".into()],
///             description: Some("A cool plugin".into()),
///             dependencies: vec![],
///         }
///     }
///
///     async fn on_enable(&self, ctx: &dyn PluginContext) -> Result<(), PluginError> {
///         // Register event listeners, commands, etc.
///         Ok(())
///     }
/// }
/// ```
pub trait Plugin: Send + Sync {
    /// Returns the plugin's metadata.
    fn metadata(&self) -> PluginMetadata;

    /// Called when the plugin is enabled (proxy startup or hot-load).
    ///
    /// Use the [`PluginContext`] to register event listeners, commands,
    /// limbo handlers, and access proxy services.
    fn on_enable(
        &self,
        ctx: &dyn PluginContext,
    ) -> impl std::future::Future<Output = Result<(), PluginError>> + Send;

    /// Called when the plugin is disabled (proxy shutdown or hot-unload).
    ///
    /// Override this to clean up resources. The default implementation
    /// does nothing.
    fn on_disable(&self) -> impl std::future::Future<Output = Result<(), PluginError>> + Send {
        async { Ok(()) }
    }
}

mod private {
    /// Sealed — only the proxy implements [`PluginContext`](super::PluginContext).
    pub trait Sealed {}
}

/// Context provided to plugins during [`Plugin::on_enable`].
///
/// Gives access to all proxy services and registration methods.
/// The proxy is the sole implementor.
pub trait PluginContext: Send + Sync + private::Sealed {
    /// Returns the event bus for subscribing to proxy events.
    fn event_bus(&self) -> &dyn EventBus;

    /// Returns the player registry for looking up connected players.
    fn player_registry(&self) -> &dyn PlayerRegistry;

    /// Returns an `Arc` handle to the player registry, suitable for
    /// capturing in closures and event handlers.
    fn player_registry_handle(&self) -> Arc<dyn PlayerRegistry>;

    /// Returns the server manager for controlling backend servers.
    fn server_manager(&self) -> &dyn ServerManager;

    /// Returns the ban service for managing bans.
    fn ban_service(&self) -> &dyn BanService;

    /// Returns the config service for reading proxy configuration.
    fn config_service(&self) -> &dyn ConfigService;

    /// Returns the command manager for registering commands.
    fn command_manager(&self) -> &dyn CommandManager;

    /// Returns the task scheduler.
    fn scheduler(&self) -> &dyn Scheduler;

    /// Registers a limbo handler for this plugin.
    ///
    /// The handler's [`name()`](LimboHandler::name) must match the name
    /// referenced in server configuration `limbo_handlers` lists.
    fn register_limbo_handler(&self, handler: Box<dyn LimboHandler>);

    /// Returns this plugin's unique ID.
    fn plugin_id(&self) -> &str;
}
