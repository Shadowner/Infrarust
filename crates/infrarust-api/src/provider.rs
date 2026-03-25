//! Plugin config provider system.
//!
//! Allows plugins to dynamically provide server configurations from
//! external sources (databases, APIs, service discovery, etc.).
//!
//! # Example
//!
//! ```ignore
//! use infrarust_api::prelude::*;
//!
//! struct MyProvider;
//!
//! impl PluginConfigProvider for MyProvider {
//!     fn provider_type(&self) -> &str { "my_api" }
//!
//!     fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ServerConfig>, PluginError>> {
//!         Box::pin(async {
//!             // Fetch configs from your source
//!             Ok(vec![])
//!         })
//!     }
//!
//!     fn watch(
//!         &self,
//!         sender: Box<dyn PluginProviderSender>,
//!     ) -> BoxFuture<'_, Result<(), PluginError>> {
//!         Box::pin(async move {
//!             while !sender.is_shutdown() {
//!                 // Poll for changes (add a delay between iterations
//!                 // to avoid busy-looping, e.g. tokio::time::sleep)
//!                 //
//!                 // sender.send(PluginProviderEvent::Added(config)).await;
//!             }
//!             Ok(())
//!         })
//!     }
//! }
//! ```

use crate::error::PluginError;
use crate::event::BoxFuture;
use crate::services::config_service::ServerConfig;
use crate::types::ServerId;

/// Event emitted by a plugin config provider when configurations change.
pub enum PluginProviderEvent {
    Added(ServerConfig),
    Updated(ServerConfig),
    Removed(ServerId),
}

/// Abstraction over the event channel used to send provider events.
///
/// The proxy provides the concrete implementation. Plugin authors
/// use this to emit config changes from their [`PluginConfigProvider::watch`]
/// implementation.
pub trait PluginProviderSender: Send + Sync {
    /// Sends an event to the proxy's config event loop.
    ///
    /// Returns `true` if the event was sent, `false` if the receiver
    /// has been dropped (proxy shutting down).
    fn send(&self, event: PluginProviderEvent) -> BoxFuture<'_, bool>;

    /// Returns `true` if the proxy has requested shutdown.
    ///
    /// Watch implementations should check this periodically and
    /// exit when it returns `true`.
    fn is_shutdown(&self) -> bool;
}

/// A source of server configurations provided by a plugin.
///
/// Implement this trait to dynamically provide server configurations
/// from external sources (e.g., a database, REST API, Kubernetes,
/// etcd, or any custom service discovery mechanism).
///
/// The proxy calls [`load_initial`](Self::load_initial) once after
/// all plugins are enabled, then spawns [`watch`](Self::watch) in
/// a background task to receive ongoing changes.
pub trait PluginConfigProvider: Send + Sync {
    /// A unique type name for this provider (e.g., `"kubernetes"`, `"database"`).
    fn provider_type(&self) -> &str;

    /// Loads the initial set of server configurations.
    ///
    /// Called once after all plugins are enabled, before the server
    /// starts accepting connections. Individual failures should be
    /// logged and skipped rather than propagated.
    fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ServerConfig>, PluginError>>;

    /// Watches for configuration changes and sends events.
    ///
    /// Runs in a background task. Use the provided [`PluginProviderSender`]
    /// to emit [`PluginProviderEvent`]s. The implementation should exit
    /// when [`PluginProviderSender::is_shutdown`] returns `true`.
    ///
    /// If the provider does not support watching (static configs only),
    /// return immediately with `Ok(())`.
    fn watch(
        &self,
        sender: Box<dyn PluginProviderSender>,
    ) -> BoxFuture<'_, Result<(), PluginError>>;
}
