use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::Instant;

use infrarust_api::plugin::{PluginContext, PluginMetadata, PluginRuntimeStatus, PluginState};

use super::error::PluginManagerError;
use super::manager::PluginManager;

#[derive(Debug, Clone)]
pub struct PluginStatus {
    pub metadata: PluginMetadata,
    pub state: Option<PluginState>,
    pub runtime: Option<PluginRuntimeStatus>,
}

#[derive(Clone)]
pub struct PluginManagerHandle(Arc<RwLock<PluginManager>>);

impl PluginManagerHandle {
    pub fn new(manager: PluginManager) -> Self {
        Self(Arc::new(RwLock::new(manager)))
    }

    pub async fn plugins(&self) -> Vec<PluginStatus> {
        let manager = self.0.read().await;
        manager
            .list_plugins()
            .into_iter()
            .map(|metadata| PluginStatus {
                metadata: metadata.clone(),
                state: manager.plugin_state(&metadata.id).cloned(),
                runtime: manager.plugin_runtime(&metadata.id),
            })
            .collect()
    }

    pub async fn plugin_count(&self) -> usize {
        self.0.read().await.list_plugins().len()
    }

    pub async fn plugin_state(&self, id: &str) -> Option<PluginState> {
        self.0.read().await.plugin_state(id).cloned()
    }

    pub async fn is_plugin_loaded(&self, id: &str) -> bool {
        self.0.read().await.is_plugin_loaded(id)
    }

    pub async fn plugin_context(&self, id: &str) -> Option<Arc<dyn PluginContext>> {
        self.0.read().await.plugin_context(id)
    }

    pub async fn disable_plugin(&self, id: &str) -> Result<(), PluginManagerError> {
        let stop = self.0.write().await.begin_plugin_stop(id)?;
        stop.run().await;
        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        self.0.write().await.shutdown().await;
    }

    pub(crate) async fn shutdown_until(&self, deadline: Instant) {
        self.0.write().await.shutdown_until(deadline).await;
    }
}
