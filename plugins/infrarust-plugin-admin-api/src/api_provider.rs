use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::provider::{PluginConfigProvider, PluginProviderSender, ServerDocument};
use tokio_util::sync::CancellationToken;

use crate::server_dir::{ProviderSenderSlot, ServerDir};

/// A `PluginConfigProvider` serving the TOML documents in `<data_dir>/servers/`.
///
/// The `watch()` method stores the `PluginProviderSender` so that REST
/// handlers and the directory watcher can emit changes at any time.
pub struct ApiConfigProvider {
    pub dir: Arc<ServerDir>,
    pub sender: Arc<ProviderSenderSlot>,
    pub shutdown: CancellationToken,
}

impl PluginConfigProvider for ApiConfigProvider {
    fn provider_type(&self) -> &str {
        "api"
    }

    fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ServerDocument>, PluginError>> {
        Box::pin(async { Ok(self.dir.initial_documents().await) })
    }

    fn watch(
        &self,
        sender: Box<dyn PluginProviderSender>,
    ) -> BoxFuture<'_, Result<(), PluginError>> {
        let sender_slot = self.sender.clone();
        let dir = self.dir.clone();
        let shutdown = self.shutdown.clone();
        Box::pin(async move {
            *sender_slot.lock().await = Some(sender);

            // The watcher had nowhere to send what happened between the
            // hand-over and now, so it is still pending here.
            crate::server_dir::reload(&dir, &sender_slot).await;

            shutdown.cancelled().await;
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
            dir: Arc::new(ServerDir::open(root.path()).unwrap()),
            sender: Arc::new(ProviderSenderSlot::new(None)),
            shutdown: CancellationToken::new(),
        };

        std::fs::write(root.path().join("servers/lobby.toml"), LOBBY).unwrap();

        let documents = provider.load_initial().await.unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].id.as_str(), "lobby");
    }

    /// Same race one step later: the file lands after the initial hand-over
    /// but before the sender is installed.
    #[tokio::test]
    async fn a_file_added_before_the_sender_is_installed_reaches_the_proxy() {
        let root = tempfile::tempdir().unwrap();
        let shutdown = CancellationToken::new();
        let provider = ApiConfigProvider {
            dir: Arc::new(ServerDir::open(root.path()).unwrap()),
            sender: Arc::new(ProviderSenderSlot::new(None)),
            shutdown: shutdown.clone(),
        };
        assert!(provider.load_initial().await.unwrap().is_empty());

        std::fs::write(root.path().join("servers/lobby.toml"), LOBBY).unwrap();
        let recorder = RecordingSender::default();

        shutdown.cancel();
        tokio::time::timeout(
            Duration::from_secs(5),
            provider.watch(Box::new(recorder.clone())),
        )
        .await
        .expect("watch returns once the plugin is shut down")
        .unwrap();

        assert_eq!(recorder.events(), vec!["added:lobby".to_string()]);
    }

    #[tokio::test]
    async fn watch_parks_until_the_plugin_shuts_down() {
        let root = tempfile::tempdir().unwrap();
        let shutdown = CancellationToken::new();
        let provider = Arc::new(ApiConfigProvider {
            dir: Arc::new(ServerDir::open(root.path()).unwrap()),
            sender: Arc::new(ProviderSenderSlot::new(None)),
            shutdown: shutdown.clone(),
        });

        let watching = {
            let provider = Arc::clone(&provider);
            tokio::spawn(async move {
                provider
                    .watch(Box::new(RecordingSender::default()))
                    .await
                    .unwrap();
            })
        };
        tokio::task::yield_now().await;
        assert!(!watching.is_finished());
        assert!(provider.sender.lock().await.is_some());

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), watching)
            .await
            .expect("watch returns once the plugin is shut down")
            .unwrap();
    }
}
