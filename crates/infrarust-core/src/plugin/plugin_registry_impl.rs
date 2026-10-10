use std::sync::{Arc, RwLock, Weak};

use infrarust_api::plugin::{Plugin, PluginMetadata, PluginState};
use infrarust_api::services::plugin_registry::{PluginInfo, PluginRegistry};

use crate::util::sync::{read, write};

struct Entry {
    metadata: PluginMetadata,
    plugin: Weak<dyn Plugin>,
}

impl Entry {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            metadata: self.metadata.clone(),
            state: PluginState::Enabled,
            runtime: self
                .plugin
                .upgrade()
                .and_then(|plugin| plugin.runtime_status()),
        }
    }
}

pub struct PluginRegistryImpl {
    data: RwLock<Vec<Entry>>,
}

impl PluginRegistryImpl {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(Vec::new()),
        }
    }

    pub fn insert_enabled(&self, metadata: &PluginMetadata, plugin: &Arc<dyn Plugin>) {
        let mut data = write(&self.data);
        data.retain(|entry| entry.metadata.id != metadata.id);
        data.push(Entry {
            metadata: metadata.clone(),
            plugin: Arc::downgrade(plugin),
        });
    }

    pub fn remove(&self, id: &str) {
        write(&self.data).retain(|entry| entry.metadata.id != id);
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
        read(&self.data).iter().map(Entry::info).collect()
    }

    fn plugin_info(&self, id: &str) -> Option<PluginInfo> {
        read(&self.data)
            .iter()
            .find(|entry| entry.metadata.id == id)
            .map(Entry::info)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use infrarust_api::error::PluginError;
    use infrarust_api::event::BoxFuture;
    use infrarust_api::plugin::{
        PluginContext, PluginHealth, PluginQueueStats, PluginRuntimeStatus, QueueWindow,
    };

    use super::*;

    struct Supervised(PluginRuntimeStatus);

    struct Native;

    impl Plugin for Native {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new("native", "Native", "1.0.0")
        }

        fn on_enable<'a>(
            &'a self,
            _ctx: &'a dyn PluginContext,
        ) -> BoxFuture<'a, Result<(), PluginError>> {
            Box::pin(async { Ok(()) })
        }
    }

    impl Plugin for Supervised {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new("supervised", "Supervised", "1.0.0")
        }

        fn on_enable<'a>(
            &'a self,
            _ctx: &'a dyn PluginContext,
        ) -> BoxFuture<'a, Result<(), PluginError>> {
            Box::pin(async { Ok(()) })
        }

        fn runtime_status(&self) -> Option<PluginRuntimeStatus> {
            Some(self.0.clone())
        }
    }

    fn quarantined() -> PluginRuntimeStatus {
        PluginRuntimeStatus::new(
            PluginHealth::Quarantined {
                retry_in: Duration::from_secs(4),
            },
            3,
            PluginQueueStats::new(0, 1024, QueueWindow::default()),
        )
    }

    #[test]
    fn a_running_plugin_reports_its_runtime_status_each_time_it_is_read() {
        let registry = PluginRegistryImpl::new();
        let plugin: Arc<dyn Plugin> = Arc::new(Supervised(quarantined()));
        registry.insert_enabled(&plugin.metadata(), &plugin);

        let info = registry.plugin_info("supervised").unwrap();
        assert_eq!(info.state, PluginState::Enabled);
        assert_eq!(info.runtime, Some(quarantined()));
        assert_eq!(registry.list_plugin_info()[0].runtime, Some(quarantined()));
    }

    #[test]
    fn a_plugin_without_a_supervised_runtime_has_no_runtime_status() {
        let registry = PluginRegistryImpl::new();
        let plugin: Arc<dyn Plugin> = Arc::new(Native);
        registry.insert_enabled(&plugin.metadata(), &plugin);
        assert_eq!(registry.plugin_info("native").unwrap().runtime, None);
    }

    #[test]
    fn a_dropped_plugin_reads_without_a_runtime_status() {
        let registry = PluginRegistryImpl::new();
        let plugin: Arc<dyn Plugin> = Arc::new(Supervised(quarantined()));
        registry.insert_enabled(&plugin.metadata(), &plugin);
        drop(plugin);
        assert_eq!(registry.plugin_info("supervised").unwrap().runtime, None);
    }
}
