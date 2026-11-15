//! [`PluginProviderSender`]s standing in for the proxy.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::BoxFuture;

use infrarust_api::provider::{PluginProviderEvent, PluginProviderSender};

use super::{DocumentId, ServerDir};

#[derive(Clone, Default)]
pub struct RecordingSender(Arc<Mutex<Vec<String>>>);

impl RecordingSender {
    pub fn events(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl PluginProviderSender for RecordingSender {
    fn send(&self, event: PluginProviderEvent) -> BoxFuture<'_, bool> {
        let label = match event {
            PluginProviderEvent::Added(doc) => format!("added:{}", doc.id.as_str()),
            PluginProviderEvent::Updated(doc) => format!("updated:{}", doc.id.as_str()),
            PluginProviderEvent::Removed(id) => format!("removed:{}", id.as_str()),
            _ => "other".to_string(),
        };
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(label);
        Box::pin(async { true })
    }

    fn is_shutdown(&self) -> bool {
        false
    }
}

pub struct SlowSender {
    delay: Duration,
}

impl SlowSender {
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }
}

impl PluginProviderSender for SlowSender {
    fn send(&self, _event: PluginProviderEvent) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            tokio::time::sleep(self.delay).await;
            true
        })
    }

    fn is_shutdown(&self) -> bool {
        false
    }
}

pub type Announcements = Arc<Mutex<Vec<(String, Option<String>)>>>;

pub struct DocumentDuringSend {
    dir: Arc<ServerDir>,
    delay: Duration,
    announcements: Announcements,
}

impl DocumentDuringSend {
    pub fn new(dir: Arc<ServerDir>, delay: Duration) -> (Self, Announcements) {
        let announcements = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                dir,
                delay,
                announcements: announcements.clone(),
            },
            announcements,
        )
    }
}

impl PluginProviderSender for DocumentDuringSend {
    fn send(&self, event: PluginProviderEvent) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let (id, announced) = match event {
                PluginProviderEvent::Added(doc) | PluginProviderEvent::Updated(doc) => {
                    (doc.id.as_str().to_string(), doc.toml)
                }
                _ => return true,
            };
            tokio::time::sleep(self.delay).await;
            let stored = DocumentId::new(&id)
                .and_then(|id| std::fs::read_to_string(self.dir.path_of(&id)).ok());
            self.announcements
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((announced, stored));
            true
        })
    }

    fn is_shutdown(&self) -> bool {
        false
    }
}
