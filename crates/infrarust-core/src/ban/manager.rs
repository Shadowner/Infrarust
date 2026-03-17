//! Ban manager: orchestrates storage, runtime kick, and periodic purge.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::ban::storage::BanStorage;
use crate::ban::types::{BanEntry, BanTarget};
use crate::error::CoreError;
use crate::registry::ConnectionRegistry;

/// High-level ban manager.
///
/// Orchestrates the storage backend, runtime kick of connected players,
/// and periodic purge of expired bans.
pub struct BanManager {
    /// The storage backend.
    storage: Arc<dyn BanStorage>,
    /// The connection registry (for runtime kick).
    connection_registry: Arc<ConnectionRegistry>,
}

impl BanManager {
    /// Creates a new ban manager.
    pub fn new(storage: Arc<dyn BanStorage>, connection_registry: Arc<ConnectionRegistry>) -> Self {
        Self {
            storage,
            connection_registry,
        }
    }

    /// Loads ban data from the storage backend.
    ///
    /// # Errors
    /// Returns `CoreError` if the storage backend fails to load.
    pub async fn load(&self) -> Result<(), CoreError> {
        self.storage.load().await
    }

    /// Bans a target. If the player is online, kicks them.
    ///
    /// 1. Adds the ban to storage
    /// 2. Looks up the player in the connection registry
    /// 3. If found, cancels their session token (kick)
    ///
    /// # Errors
    /// Returns `CoreError` if the storage backend fails to add the ban.
    pub async fn ban(
        &self,
        target: BanTarget,
        reason: Option<String>,
        duration: Option<Duration>,
        source: String,
    ) -> Result<(), CoreError> {
        let entry = BanEntry::new(target.clone(), reason, duration, source);
        self.storage.add_ban(entry).await?;

        // Kick connected player(s) matching this target
        let sessions_to_kick = match &target {
            BanTarget::Ip(ip) => self.connection_registry.find_by_ip(ip),
            BanTarget::Username(name) => self
                .connection_registry
                .find_by_username(name)
                .into_iter()
                .collect(),
            BanTarget::Uuid(uuid) => self
                .connection_registry
                .find_by_uuid(uuid)
                .into_iter()
                .collect(),
        };

        for session in &sessions_to_kick {
            tracing::info!(
                ban_target = %target,
                username = ?session.username,
                "kicking connected player due to ban"
            );
            session.shutdown_token.cancel();
        }

        if !sessions_to_kick.is_empty() {
            tracing::info!(
                ban_target = %target,
                kicked = sessions_to_kick.len(),
                "banned and kicked online player(s)"
            );
        }

        Ok(())
    }

    /// Lifts a ban. Returns `true` if a ban existed.
    ///
    /// # Errors
    /// Returns `CoreError` if the storage backend fails.
    pub async fn unban(&self, target: &BanTarget) -> Result<bool, CoreError> {
        self.storage.remove_ban(target).await
    }

    /// Checks if an IP is banned (called by `BanIpCheckMiddleware` in the common pipeline).
    ///
    /// # Errors
    /// Returns `CoreError` if the storage backend fails.
    pub async fn is_ip_banned(&self, ip: &IpAddr) -> Result<Option<BanEntry>, CoreError> {
        self.storage.is_banned(&BanTarget::Ip(*ip)).await
    }

    /// Checks a player against the ban storage (called by `BanCheckMiddleware`).
    ///
    /// # Errors
    /// Returns `CoreError` if the storage backend fails.
    pub async fn check_player(
        &self,
        ip: &IpAddr,
        username: &str,
        uuid: Option<&Uuid>,
    ) -> Result<Option<BanEntry>, CoreError> {
        self.storage.check_player(ip, username, uuid).await
    }

    /// Lists all active bans.
    ///
    /// # Errors
    /// Returns `CoreError` if the storage backend fails.
    pub async fn get_all_bans(&self) -> Result<Vec<BanEntry>, CoreError> {
        self.storage.get_all_active().await
    }

    /// Starts the periodic purge task.
    /// Returns a `JoinHandle` that can be awaited on shutdown.
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
                    _ = ticker.tick() => {
                        match storage.purge_expired().await {
                            Ok(0) => {}
                            Ok(n) => tracing::debug!(count = n, "purged expired bans"),
                            Err(e) => tracing::warn!(error = %e, "failed to purge expired bans"),
                        }
                    }
                    () = shutdown.cancelled() => {
                        tracing::debug!("ban purge task stopped");
                        break;
                    }
                }
            }
        })
    }
}
