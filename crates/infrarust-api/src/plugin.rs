//! Plugin lifecycle traits and metadata.
//!
//! The [`Plugin`] trait is the entry point for all Infrarust plugins.
//! Plugins register event listeners, commands, and handlers during
//! [`on_enable`](Plugin::on_enable) via the [`PluginContext`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::command::CommandManager;
use crate::error::PluginError;
use crate::event::BoxFuture;
use crate::event::bus::EventBus;
use crate::filter::registry::{CodecFilterRegistry, TransportFilterRegistry};
use crate::limbo::{LimboHandler, LimboHandlerError, LimboHandlerRegistration};
use crate::permissions::{
    PermissionNode, PermissionNodeError, PermissionNodeInfo, PermissionProvider,
};
use crate::services::{
    ban_service::{BanProvider, BanService},
    config_service::ConfigService,
    load_balancer::LoadBalancerService,
    player_registry::PlayerRegistry,
    plugin_registry::PluginRegistry,
    providers::ProviderRejected,
    proxy_info::ProxyInfo,
    scheduler::Scheduler,
    server_manager::ServerManager,
    service_registry::ServiceRegistry,
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

/// Tracks the lifecycle state of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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

impl PluginState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginHealth {
    Healthy,
    Recovering { retry_in: Option<Duration> },
    Quarantined { retry_in: Duration },
    Stopped,
}

impl PluginHealth {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Recovering { .. } => "recovering",
            Self::Quarantined { .. } => "quarantined",
            Self::Stopped => "stopped",
        }
    }

    pub const fn retry_in(&self) -> Option<Duration> {
        match self {
            Self::Recovering { retry_in } => *retry_in,
            Self::Quarantined { retry_in } => Some(*retry_in),
            Self::Healthy | Self::Stopped => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueueWindow {
    pub span: Duration,
    pub taken: u64,
    pub peak_depth: usize,
    pub wait_p50: Duration,
    pub wait_p99: Duration,
    pub wait_max: Duration,
}

impl QueueWindow {
    pub const fn new(span: Duration, taken: u64, peak_depth: usize) -> Self {
        Self {
            span,
            taken,
            peak_depth,
            wait_p50: Duration::ZERO,
            wait_p99: Duration::ZERO,
            wait_max: Duration::ZERO,
        }
    }

    #[must_use]
    pub const fn waits(mut self, p50: Duration, p99: Duration, max: Duration) -> Self {
        self.wait_p50 = p50;
        self.wait_p99 = p99;
        self.wait_max = max;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginQueueStats {
    pub depth: usize,
    pub capacity: usize,
    pub recent: QueueWindow,
}

impl PluginQueueStats {
    pub const fn new(depth: usize, capacity: usize, recent: QueueWindow) -> Self {
        Self {
            depth,
            capacity,
            recent,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginRestarts {
    pub in_window: u32,
    pub max: u32,
    pub window: Duration,
}

impl PluginRestarts {
    pub const fn new(in_window: u32, max: u32, window: Duration) -> Self {
        Self {
            in_window,
            max,
            window,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginFault {
    pub cause: String,
    pub ago: Duration,
    pub generation: u64,
}

impl PluginFault {
    pub fn new(cause: impl Into<String>, ago: Duration, generation: u64) -> Self {
        Self {
            cause: cause.into(),
            ago,
            generation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginRuntimeStatus {
    pub health: PluginHealth,
    pub generation: u64,
    pub queue: PluginQueueStats,
    pub restarts: PluginRestarts,
    pub last_fault: Option<PluginFault>,
}

impl PluginRuntimeStatus {
    pub const fn new(health: PluginHealth, generation: u64, queue: PluginQueueStats) -> Self {
        Self {
            health,
            generation,
            queue,
            restarts: PluginRestarts::new(0, 0, Duration::ZERO),
            last_fault: None,
        }
    }

    #[must_use]
    pub const fn with_restarts(mut self, restarts: PluginRestarts) -> Self {
        self.restarts = restarts;
        self
    }

    #[must_use]
    pub fn with_last_fault(mut self, fault: PluginFault) -> Self {
        self.last_fault = Some(fault);
        self
    }
}

/// A dependency on another plugin.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginDependency {
    /// The ID of the required plugin.
    pub id: String,
    /// If `true`, the plugin can function without this dependency.
    pub optional: bool,
}

impl PluginDependency {
    pub fn new(id: impl Into<String>, optional: bool) -> Self {
        Self {
            id: id.into(),
            optional,
        }
    }
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

    fn runtime_status(&self) -> Option<PluginRuntimeStatus> {
        None
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
        self.dependencies.push(PluginDependency::new(id, false));
        self
    }

    /// Adds an optional dependency.
    pub fn optional_dependency(mut self, id: impl Into<String>) -> Self {
        self.dependencies.push(PluginDependency::new(id, true));
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
    fn event_bus(&self) -> Arc<dyn EventBus>;

    fn player_registry(&self) -> Arc<dyn PlayerRegistry>;

    fn server_manager(&self) -> Arc<dyn ServerManager>;

    fn ban_service(&self) -> Arc<dyn BanService>;

    fn register_ban_provider(&self, provider: Arc<dyn BanProvider>)
    -> Result<(), ProviderRejected>;

    fn register_permission_provider(
        &self,
        provider: Arc<dyn PermissionProvider>,
    ) -> Result<(), ProviderRejected>;

    fn register_permission_node(&self, node: PermissionNode) -> Result<(), PermissionNodeError>;

    fn permission_nodes(&self) -> Vec<PermissionNodeInfo>;

    fn config_service(&self) -> Arc<dyn ConfigService>;

    fn load_balancer_service(&self) -> Arc<dyn LoadBalancerService>;

    fn command_manager(&self) -> Arc<dyn CommandManager>;

    fn scheduler(&self) -> Arc<dyn Scheduler>;

    fn services(&self) -> Arc<dyn ServiceRegistry>;

    /// Registers a limbo handler for this plugin.
    ///
    /// The handler's [`name()`](LimboHandler::name) must match the name
    /// referenced in server configuration `limbo_handlers` lists.
    fn register_limbo_handler(
        &self,
        handler: Box<dyn LimboHandler>,
    ) -> Result<LimboHandlerRegistration, LimboHandlerError>;

    /// Returns the codec filter registry for registering packet-level filters.
    ///
    /// Returns `Some` only if the plugin holds [`Capability::CodecFilter`], else `None`.
    ///
    /// [`Capability::CodecFilter`]: crate::permissions::Capability::CodecFilter
    fn codec_filters(&self) -> Option<&dyn CodecFilterRegistry>;

    /// Returns the transport filter registry for registering TCP-level filters.
    ///
    /// Returns `Some` only if the plugin holds [`Capability::TransportFilter`], else
    /// `None`. That capability is granted to trusted native plugins only.
    ///
    /// [`Capability::TransportFilter`]: crate::permissions::Capability::TransportFilter
    fn transport_filters(&self) -> Option<&dyn TransportFilterRegistry>;

    fn plugin_registry(&self) -> Arc<dyn PluginRegistry>;

    fn register_config_provider(&self, provider: Box<dyn crate::provider::PluginConfigProvider>);

    fn plugin_id(&self) -> &str;

    fn data_dir(&self) -> PathBuf;

    fn proxy_shutdown(&self) -> CancellationToken;

    fn proxy_info(&self) -> &ProxyInfo;

    /// Capabilities granted to this plugin (source: Infrarust config).
    fn capabilities(&self) -> &crate::permissions::CapabilitySet;

    fn channel_registrar(&self) -> &dyn crate::messaging::ChannelRegistrar;

    fn server_messenger(&self) -> Arc<dyn crate::messaging::ServerMessenger>;
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
