//! [`BanService`] bridge — delegates to the internal [`BanManager`].

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{BanEntry, BanService, BanTarget};

use crate::ban::manager::BanManager;
use crate::ban::types as core_ban;

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
                .ban(
                    to_core_target(&target),
                    reason,
                    duration,
                    "plugin".to_string(),
                )
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn unban(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>> {
        let core_target = to_core_target(target);
        Box::pin(async move {
            self.manager
                .unban(&core_target)
                .await
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn is_banned(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>> {
        let core_target = to_core_target(target);
        Box::pin(async move {
            self.manager
                .is_banned(&core_target)
                .await
                .map(|entry| entry.is_some())
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn get_ban(&self, target: &BanTarget) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        let core_target = to_core_target(target);
        Box::pin(async move {
            self.manager
                .is_banned(&core_target)
                .await
                .map(|entry| entry.map(|e| from_core_entry(&e)))
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }

    fn get_all_bans(&self) -> BoxFuture<'_, Result<Vec<BanEntry>, ServiceError>> {
        Box::pin(async move {
            self.manager
                .get_all_bans()
                .await
                .map(|bans| bans.iter().map(from_core_entry).collect())
                .map_err(|e| ServiceError::OperationFailed(e.to_string()))
        })
    }
}

/// Converts an API ban target to a core ban target.
fn to_core_target(target: &BanTarget) -> core_ban::BanTarget {
    match target {
        BanTarget::Ip(ip) => core_ban::BanTarget::Ip(*ip),
        BanTarget::Username(name) => core_ban::BanTarget::Username(name.clone()),
        BanTarget::Uuid(uuid) => core_ban::BanTarget::Uuid(*uuid),
        _ => core_ban::BanTarget::Username(String::new()),
    }
}

/// Converts a core ban entry to an API ban entry.
fn from_core_entry(entry: &core_ban::BanEntry) -> BanEntry {
    BanEntry {
        target: from_core_target(&entry.target),
        reason: entry.reason.clone(),
        expires_at: entry.expires_at,
        created_at: entry.created_at,
        source: entry.source.clone(),
    }
}

/// Converts a core ban target to an API ban target.
fn from_core_target(target: &core_ban::BanTarget) -> BanTarget {
    match target {
        core_ban::BanTarget::Ip(ip) => BanTarget::Ip(*ip),
        core_ban::BanTarget::Username(name) => BanTarget::Username(name.clone()),
        core_ban::BanTarget::Uuid(uuid) => BanTarget::Uuid(*uuid),
    }
}
