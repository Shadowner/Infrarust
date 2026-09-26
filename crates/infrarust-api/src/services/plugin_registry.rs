//! Plugin registry trait — read-only view of loaded plugins.

use crate::plugin::{PluginMetadata, PluginState};

pub mod private {
    pub trait Sealed {}
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub metadata: PluginMetadata,
    pub state: PluginState,
}

impl PluginInfo {
    pub fn id(&self) -> &str {
        &self.metadata.id
    }
}

/// Read-only view of all loaded plugins.
pub trait PluginRegistry: Send + Sync + private::Sealed {
    fn list_plugin_info(&self) -> Vec<PluginInfo>;
    fn plugin_info(&self, id: &str) -> Option<PluginInfo>;
}
