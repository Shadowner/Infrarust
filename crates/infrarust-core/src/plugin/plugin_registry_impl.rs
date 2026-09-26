use std::sync::RwLock;

use infrarust_api::plugin::PluginMetadata;
use infrarust_api::services::plugin_registry::{PluginDependencyInfo, PluginInfo, PluginRegistry};

const ENABLED: &str = "enabled";

pub struct PluginRegistryImpl {
    data: RwLock<Vec<PluginInfo>>,
}

impl PluginRegistryImpl {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(Vec::new()),
        }
    }

    pub fn insert_enabled(&self, metadata: &PluginMetadata) {
        let info = PluginInfo {
            id: metadata.id.clone(),
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            authors: metadata.authors.clone(),
            description: metadata.description.clone(),
            state: ENABLED.to_string(),
            dependencies: metadata
                .dependencies
                .iter()
                .map(|d| PluginDependencyInfo {
                    id: d.id.clone(),
                    optional: d.optional,
                })
                .collect(),
        };
        let mut data = self.data.write().expect("lock poisoned");
        data.retain(|p| p.id != info.id);
        data.push(info);
    }

    pub fn remove(&self, id: &str) {
        self.data
            .write()
            .expect("lock poisoned")
            .retain(|p| p.id != id);
    }
}

impl Default for PluginRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl infrarust_api::services::plugin_registry::private::Sealed for PluginRegistryImpl {}

impl PluginRegistry for PluginRegistryImpl {
    fn list_plugin_info(&self) -> Vec<PluginInfo> {
        self.data.read().expect("lock poisoned").clone()
    }

    fn plugin_info(&self, id: &str) -> Option<PluginInfo> {
        self.data
            .read()
            .expect("lock poisoned")
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }
}
