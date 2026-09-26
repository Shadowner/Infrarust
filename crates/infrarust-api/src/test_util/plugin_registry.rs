use std::sync::Mutex;

use crate::services::plugin_registry::{PluginInfo, PluginRegistry};

use super::lock;

#[derive(Debug, Default)]
pub struct MockPluginRegistry {
    plugins: Mutex<Vec<PluginInfo>>,
}

impl MockPluginRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_plugin(self, info: PluginInfo) -> Self {
        let mut plugins = lock(&self.plugins);
        plugins.retain(|p| p.id != info.id);
        plugins.push(info);
        drop(plugins);
        self
    }
}

impl crate::services::plugin_registry::private::Sealed for MockPluginRegistry {}

impl PluginRegistry for MockPluginRegistry {
    fn list_plugin_info(&self) -> Vec<PluginInfo> {
        lock(&self.plugins).clone()
    }

    fn plugin_info(&self, id: &str) -> Option<PluginInfo> {
        lock(&self.plugins).iter().find(|p| p.id == id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_finds_registered_plugins() {
        let info = PluginInfo {
            id: "hello".into(),
            name: "Hello".into(),
            version: "1.0.0".into(),
            authors: vec![],
            description: None,
            state: "enabled".into(),
            dependencies: vec![],
        };
        let registry = MockPluginRegistry::new().with_plugin(info);
        assert_eq!(registry.list_plugin_info().len(), 1);
        assert!(registry.plugin_info("hello").is_some());
        assert!(registry.plugin_info("other").is_none());
    }
}
