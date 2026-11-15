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

pub trait ConfigProvider: Send + Sync {
    fn provider_type(&self) -> &str;

    fn load_initial(&self) -> BoxFuture<'_, Result<Vec<ProviderConfig>, CoreError>>;

    fn watch(
        &self,
        sender: mpsc::Sender<Vec<ProviderChange>>,
        shutdown: CancellationToken,
    ) -> BoxFuture<'_, Result<(), CoreError>>;
}

pub trait RoutesListener: Send + Sync {
    fn routes_changed(&self) -> BoxFuture<'_, ()>;
}
