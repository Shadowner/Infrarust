use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::provider::{PluginConfigProvider, PluginProviderSender};
use infrarust_api::services::config_service::ServerConfig;
use tokio::sync::Mutex;

use crate::server_store::ApiServerStore;

/// A `PluginConfigProvider` that serves API-created servers.
///
/// The `watch()` method stores the `PluginProviderSender` so that
/// REST handlers can emit `Added`/`Updated`/`Removed` events at any time.
pub struct ApiConfigProvider {
    pub store: Arc<ApiServerStore>,
    pub sender: Arc<Mutex<Option<Box<dyn PluginProviderSender>>>>,
}

impl PluginConfigProvider for ApiConfigProvider {
    fn provider_type(&self) -> &str {
        "api"
    }

    fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ServerConfig>, PluginError>> {
        Box::pin(async { Ok(self.store.all()) })
    }

    fn watch(
        &self,
        sender: Box<dyn PluginProviderSender>,
    ) -> BoxFuture<'_, Result<(), PluginError>> {
        let sender_slot = self.sender.clone();
        Box::pin(async move {
            *sender_slot.lock().await = Some(sender);

            // Park until proxy shutdown — the sender is used by REST handlers.
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let guard = sender_slot.lock().await;
                if let Some(s) = guard.as_ref() {
                    if s.is_shutdown() {
                        break;
                    }
                } else {
                    break;
                }
            }

            Ok(())
        })
    }
}
