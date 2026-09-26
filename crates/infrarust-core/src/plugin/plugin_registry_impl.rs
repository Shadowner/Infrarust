use std::sync::RwLock;

use infrarust_api::plugin::{PluginMetadata, PluginState};
use infrarust_api::services::plugin_registry::{PluginInfo, PluginRegistry};

use crate::util::sync::{read, write};

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
            metadata: metadata.clone(),
            state: PluginState::Enabled,
        };
        let mut data = write(&self.data);
        data.retain(|p| p.id() != info.id());
        data.push(info);
    }

    pub fn remove(&self, id: &str) {
        write(&self.data).retain(|p| p.id() != id);
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
        read(&self.data).clone()
    }

    fn plugin_info(&self, id: &str) -> Option<PluginInfo> {
        read(&self.data).iter().find(|p| p.id() == id).cloned()
    }
}
