use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::provider::{PluginConfigProvider, PluginProviderSender, ServerDocument};
use infrarust_api::services::config_service::ServerConfig;

use crate::server_dir::{ProviderSenderSlot, ServerDir};

/// A `PluginConfigProvider` serving the TOML documents in `<data_dir>/servers/`.
///
/// The `watch()` method stores the `PluginProviderSender` so that REST
/// handlers and the directory watcher can emit changes at any time.
pub struct ApiConfigProvider {
    pub dir: Arc<ServerDir>,
    pub sender: Arc<ProviderSenderSlot>,
}

impl PluginConfigProvider for ApiConfigProvider {
    fn provider_type(&self) -> &str {
        "api"
    }

    fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ServerConfig>, PluginError>> {
        Box::pin(async { Ok(vec![]) })
    }

    fn load_initial_documents(&self) -> BoxFuture<'_, Result<Vec<ServerDocument>, PluginError>> {
        Box::pin(async { Ok(self.dir.initial_documents().await) })
    }

    fn watch(
        &self,
        sender: Box<dyn PluginProviderSender>,
    ) -> BoxFuture<'_, Result<(), PluginError>> {
        let sender_slot = self.sender.clone();
        let dir = self.dir.clone();
        Box::pin(async move {
            *sender_slot.lock().await = Some(sender);

            // The watcher had nowhere to send what happened between the
            // hand-over and now, so it is still pending here.
            crate::server_dir::reload(&dir, &sender_slot).await;

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::time::Duration;

    use super::*;
    use crate::server_dir::test_support::RecordingSender;

    const LOBBY: &str = "domains = [\"lobby.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n";

    /// The watcher fires while the sender slot is still empty, and no second
    /// event ever comes for the same file.
    #[tokio::test]
    async fn a_file_added_before_the_hand_over_reaches_the_proxy() {
        let root = tempfile::tempdir().unwrap();
        let provider = ApiConfigProvider {
            dir: Arc::new(ServerDir::open(root.path())),
            sender: Arc::new(ProviderSenderSlot::new(None)),
        };

        std::fs::write(root.path().join("servers/lobby.toml"), LOBBY).unwrap();

        let documents = provider.load_initial_documents().await.unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].id.as_str(), "lobby");
    }

    /// Same race one step later: the file lands after the initial hand-over
    /// but before the sender is installed.
    #[tokio::test]
    async fn a_file_added_before_the_sender_is_installed_reaches_the_proxy() {
        let root = tempfile::tempdir().unwrap();
        let provider = ApiConfigProvider {
            dir: Arc::new(ServerDir::open(root.path())),
            sender: Arc::new(ProviderSenderSlot::new(None)),
        };
        assert!(provider.load_initial_documents().await.unwrap().is_empty());

        std::fs::write(root.path().join("servers/lobby.toml"), LOBBY).unwrap();
        let recorder = RecordingSender::default();

        // `watch` parks until shutdown, so it is only run long enough to
        // install the sender and drain what was pending.
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            provider.watch(Box::new(recorder.clone())),
        )
        .await;

        assert_eq!(recorder.events(), vec!["added:lobby".to_string()]);
    }
}
