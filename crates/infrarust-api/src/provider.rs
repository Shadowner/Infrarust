//! Plugin config provider system.
//!
//! Allows plugins to dynamically provide server configurations from
//! external sources (databases, APIs, service discovery, etc.).
//!
//! Configurations can be supplied either as the projected [`ServerConfig`],
//! which only carries the fields this crate models, or as a [`ServerDocument`]
//! holding raw TOML, which the proxy parses with its own full config schema.
//! Documents are the only way to reach fields such as load balancing, MOTD or
//! health probing.
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
//!     fn load_initial_documents(
//!         &self,
//!     ) -> BoxFuture<'_, Result<Vec<ServerDocument>, PluginError>> {
//!         Box::pin(async {
//!             Ok(vec![ServerDocument {
//!                 id: ServerId::new("survival"),
//!                 toml: r#"addresses = ["10.0.0.1:25565"]"#.to_string(),
//!             }])
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

/// A server configuration as raw TOML, parsed by the proxy against its
/// full configuration schema.
///
/// `id` identifies the document within the provider and becomes the server id
/// when the TOML itself does not set one.
#[derive(Debug, Clone)]
pub struct ServerDocument {
    pub id: ServerId,
    pub toml: String,
}

/// Event emitted by a plugin config provider when configurations change.
#[non_exhaustive]
pub enum PluginProviderEvent {
    Added(ServerConfig),
    Updated(ServerConfig),
    Removed(ServerId),
    AddedDocument(ServerDocument),
    UpdatedDocument(ServerDocument),
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

    /// Loads the initial set of raw TOML server configurations.
    ///
    /// Called alongside [`load_initial`](Self::load_initial); a provider may
    /// use either or both. Documents that fail to parse or validate are
    /// logged and skipped by the proxy.
    fn load_initial_documents(&self) -> BoxFuture<'_, Result<Vec<ServerDocument>, PluginError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

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
