//! Plugin registry trait — read-only view of loaded plugins.

pub mod private {
    pub trait Sealed {}
}

#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub state: String,
    pub dependencies: Vec<PluginDependencyInfo>,
}

#[derive(Debug, Clone)]
pub struct PluginDependencyInfo {
    pub id: String,
    pub optional: bool,
}

/// Read-only view of all loaded plugins.
pub trait PluginRegistry: Send + Sync + private::Sealed {
    fn list_plugin_info(&self) -> Vec<PluginInfo>;
    fn plugin_info(&self, id: &str) -> Option<PluginInfo>;
}
