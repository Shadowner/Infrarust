use std::sync::RwLock;

use infrarust_api::plugin::PluginMetadata;
use infrarust_api::services::plugin_registry::{PluginDependencyInfo, PluginInfo, PluginRegistry};

use super::PluginState;

pub struct PluginRegistryImpl {
    data: RwLock<Vec<PluginInfo>>,
}

impl PluginRegistryImpl {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(Vec::new()),
        }
    }

    pub fn update_from(
        &self,
        plugins: &[&PluginMetadata],
        states: &dyn Fn(&str) -> Option<PluginState>,
    ) {
        let infos = plugins
            .iter()
            .map(|meta| {
                let state = states(&meta.id)
                    .map(|s| match s {
                        PluginState::Loading => "loading".to_string(),
                        PluginState::Enabled => "enabled".to_string(),
                        PluginState::Disabled => "disabled".to_string(),
                        PluginState::Error(e) => format!("error: {e}"),
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                PluginInfo {
                    id: meta.id.clone(),
                    name: meta.name.clone(),
                    version: meta.version.clone(),
                    authors: meta.authors.clone(),
                    description: meta.description.clone(),
                    state,
                    dependencies: meta
                        .dependencies
                        .iter()
                        .map(|d| PluginDependencyInfo {
                            id: d.id.clone(),
                            optional: d.optional,
                        })
                        .collect(),
                }
            })
            .collect();

        *self.data.write().expect("lock poisoned") = infos;
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
