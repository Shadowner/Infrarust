use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{
    BanFeatures, BanPage, BanProvider, BanQuery, BanRequest, BanVerdict, LoginAttempt, UnbanRequest,
};

use crate::ban::storage::BanStorage;
use crate::ban::types::{BanEntry, BanSource, BanTarget};
use crate::error::CoreError;

pub struct BuiltinBanProvider {
    storage: Arc<dyn BanStorage>,
}

fn failed(error: CoreError) -> ServiceError {
    ServiceError::OperationFailed(error.to_string())
}

impl BuiltinBanProvider {
    pub fn new(storage: Arc<dyn BanStorage>) -> Self {
        Self { storage }
    }

    pub fn storage(&self) -> &Arc<dyn BanStorage> {
        &self.storage
    }

    pub async fn load(&self) -> Result<(), CoreError> {
        self.storage.load().await
    }

    pub fn start_purge_task(
        &self,
        interval: Duration,
        shutdown: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let storage = Arc::clone(&self.storage);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => {
                        tracing::debug!("ban purge task stopped");
                        break;
                    }
                    _ = ticker.tick() => {
                        match storage.purge_expired().await {
                            Ok(0) => {}
                            Ok(n) => tracing::debug!(count = n, "purged expired bans"),
                            Err(e) => tracing::warn!(error = %e, "failed to purge expired bans"),
                        }
                    }
                }
            }
        })
    }
}

impl BanProvider for BuiltinBanProvider {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        Box::pin(async move {
            self.storage
                .check(attempt)
                .await
                .map(|entry| entry.map(BanVerdict::new))
                .map_err(failed)
        })
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        Box::pin(async move {
            let source = request.source.unwrap_or(BanSource::System);
            let mut entry = BanEntry::new(String::new(), request.target, source);
            entry.reason = request.reason;
            if let Some(duration) = request.duration {
                entry = entry.lasting(duration);
            }
            self.storage.add_ban(entry).await.map_err(failed)
        })
    }

    fn unban(
        &self,
        request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async move {
            let source = request.source.unwrap_or(BanSource::System);
            self.storage
                .remove_ban(&request.target, &source)
                .await
                .map_err(failed)
        })
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async move { self.storage.get_ban(target).await.map_err(failed) })
    }

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        Box::pin(async move { self.storage.list(query).await.map_err(failed) })
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(true).pagination(true)
    }
}
