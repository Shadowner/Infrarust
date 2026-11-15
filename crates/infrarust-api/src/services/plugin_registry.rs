//! Plugin registry trait — read-only view of loaded plugins.

use crate::plugin::{PluginMetadata, PluginRuntimeStatus, PluginState};

#[doc(hidden)]
pub mod private {
    pub use crate::__sealed::Sealed;
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub metadata: PluginMetadata,
    pub state: PluginState,
    pub runtime: Option<PluginRuntimeStatus>,
}

impl PluginInfo {
    pub fn id(&self) -> &str {
        &self.metadata.id
    }
}

/// Read-only view of all loaded plugins.
pub trait PluginRegistry: Send + Sync + crate::__sealed::Sealed {
    fn list_plugin_info(&self) -> Vec<PluginInfo>;
    fn plugin_info(&self, id: &str) -> Option<PluginInfo>;

    fn is_loaded(&self, id: &str) -> bool {
        self.plugin_info(id).is_some()
    }
}
