use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::BoxFuture;
use infrarust_config::ServerConfig;

use crate::error::CoreError;
use crate::provider::ProviderId;

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub id: ProviderId,
    pub config: ServerConfig,
}

#[derive(Debug)]
pub enum ProviderChange {
    Added(ProviderConfig),
    Updated(ProviderConfig),
    Removed(ProviderId),
}

#[derive(Debug)]
pub enum ProviderEvent {
    Added(ProviderConfig),
    Updated(ProviderConfig),
    Removed(ProviderId),
    Batch(Vec<ProviderChange>),
}

impl ProviderEvent {
    pub fn into_changes(self) -> Vec<ProviderChange> {
        match self {
            Self::Added(config) => vec![ProviderChange::Added(config)],
            Self::Updated(config) => vec![ProviderChange::Updated(config)],
            Self::Removed(id) => vec![ProviderChange::Removed(id)],
            Self::Batch(changes) => changes,
        }
    }
}

impl From<ProviderChange> for ProviderEvent {
    fn from(change: ProviderChange) -> Self {
        match change {
            ProviderChange::Added(config) => Self::Added(config),
            ProviderChange::Updated(config) => Self::Updated(config),
            ProviderChange::Removed(id) => Self::Removed(id),
        }
    }
}

pub trait ConfigProvider: Send + Sync {
    fn provider_type(&self) -> &str;

    fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ProviderConfig>, CoreError>>;

    fn watch(
        &self,
        sender: mpsc::Sender<ProviderEvent>,
        shutdown: CancellationToken,
    ) -> BoxFuture<'_, Result<(), CoreError>>;
}

pub trait RoutesListener: Send + Sync {
    fn routes_changed(&self) -> BoxFuture<'_, ()>;
}
