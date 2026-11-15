use std::sync::Arc;
use std::time::Duration;

use infrarust_api::provider::PluginProviderSender;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::store::{DirChange, ServerDir};

const DEBOUNCE: Duration = Duration::from_millis(200);

/// Slot holding the sender the proxy hands to [`crate::api_provider::ApiConfigProvider::watch`].
pub type ProviderSenderSlot = tokio::sync::Mutex<Option<Box<dyn PluginProviderSender>>>;

#[derive(Debug, Default, Serialize)]
pub struct ReloadSummary {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// Rescans the directory and forwards every change to the proxy, or `None`
/// when the proxy's sender is not connected yet.
///
/// The filesystem watcher and `POST /api/v1/config/reload` both go through
/// here. Nothing is diffed while the sender is missing, so the changes stay
/// pending instead of being consumed by a rescan nobody hears.
pub async fn reload(dir: &ServerDir, sender: &ProviderSenderSlot) -> Option<ReloadSummary> {
    if sender.lock().await.is_none() {
        tracing::warn!("Config provider is not connected yet, skipping rescan");
        return None;
    }

    let changes = dir.diff().await;
    let mut summary = ReloadSummary::default();

    let guard = sender.lock().await;
    let sender = guard.as_ref()?;

    for change in changes {
        match &change {
            DirChange::Added(_) => summary.added += 1,
            DirChange::Updated(_) => summary.updated += 1,
            DirChange::Removed(_) => summary.removed += 1,
        }
        sender.send(change.into_event()).await;
    }

    Some(summary)
}

/// Watches the servers directory and reloads on every debounced burst.
pub async fn watch(
    dir: Arc<ServerDir>,
    sender: Arc<ProviderSenderSlot>,
    shutdown: CancellationToken,
) {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<()>();

    let watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            use notify::EventKind;
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                    let _ = notify_tx.send(());
                }
                _ => {}
            }
        }
    })
    .and_then(|mut watcher: RecommendedWatcher| {
        watcher.watch(dir.path(), RecursiveMode::NonRecursive)?;
        Ok(watcher)
    });

    let _watcher = match watcher {
        Ok(watcher) => watcher,
        Err(e) => {
            tracing::error!(
                dir = %dir.path().display(),
                error = %e,
                "Failed to watch servers directory, only manual reloads will apply"
            );
            return;
        }
    };

    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            recv = notify_rx.recv() => {
                if recv.is_none() {
                    break;
                }
                tokio::time::sleep(DEBOUNCE).await;
                while notify_rx.try_recv().is_ok() {}

                if let Some(summary) = reload(&dir, &sender).await
                    && summary.added + summary.updated + summary.removed > 0
                {
                    tracing::info!(?summary, "Servers directory changed");
                }
            }
        }
    }
}
