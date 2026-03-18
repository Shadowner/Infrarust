//! [`BanService`] bridge — delegates to the internal [`BanManager`].

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{BanEntry, BanService, BanTarget};

use crate::ban::manager::BanManager;

/// Bridges the API-level [`BanService`] trait to the core [`BanManager`].
pub struct BanServiceBridge {
    manager: Arc<BanManager>,
}

impl BanServiceBridge {
    /// Creates a new bridge.
    pub fn new(manager: Arc<BanManager>) -> Self {
        Self { manager }
    }
}

impl infrarust_api::services::ban_service::private::Sealed for BanServiceBridge {}

impl BanService for BanServiceBridge {
    fn ban(
        &self,
        target: BanTarget,
        reason: Option<String>,
        duration: Option<Duration>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            self.manager
                .ban(target, reason, duration, "plugin".to_string())
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn unban(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>> {
        let target = target.clone();
        Box::pin(async move {
            self.manager
                .unban(&target)
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn is_banned(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>> {
        let target = target.clone();
        Box::pin(async move {
            self.manager
                .is_banned(&target)
                .await
                .map(|entry| entry.is_some())
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn get_ban(&self, target: &BanTarget) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        let target = target.clone();
        Box::pin(async move {
            self.manager
                .is_banned(&target)
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn get_all_bans(&self) -> BoxFuture<'_, Result<Vec<BanEntry>, ServiceError>> {
        Box::pin(async move {
            self.manager
                .get_all_bans()
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }
}
