//! Plugin context and lifecycle management.

pub mod context;
pub mod dependency;
pub mod manager;
pub mod tracking;

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
