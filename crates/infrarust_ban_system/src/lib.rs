//! Enhanced ban system with scalable storage backends and efficient lookups.

use std::{
    net::IpAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

pub mod file;
pub mod index;
pub mod memory;
pub mod storage;

// #[cfg(feature = "redis")]
// pub mod redis;

// #[cfg(feature = "database")]
// pub mod database;

use storage::BanStorage;
pub use storage::BanStorageBackend;

#[derive(Debug, Error)]
pub enum BanError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Invalid ban entry: {0}")]
    InvalidEntry(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Ban not found")]
    NotFound,

    #[error("Ban already exists")]
    AlreadyExists,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BanEntry {
    pub id: String,
    pub ip: Option<IpAddr>,
    pub uuid: Option<String>,
    pub username: Option<String>,
    pub reason: String,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub banned_by: String,
}

impl BanEntry {
    pub fn new(
        ip: Option<IpAddr>,
        uuid: Option<String>,
        username: Option<String>,
        reason: String,
        expires_in: Option<Duration>,
        banned_by: String,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let expires_at = expires_in.map(|d| now + d.as_secs());

        Self {
            id: Uuid::new_v4().to_string(),
            ip,
            uuid,
            username,
            reason,
            created_at: now,
            expires_at,
            banned_by,
        }
    }

    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            now >= expires_at
        } else {
            false
        }
    }

    pub fn matches_ip(&self, ip: &IpAddr) -> bool {
        if let Some(banned_ip) = self.ip {
            &banned_ip == ip
        } else {
            false
        }
    }

    pub fn matches_uuid(&self, uuid: &str) -> bool {
        if let Some(banned_uuid) = &self.uuid {
            banned_uuid == uuid
        } else {
            false
        }
    }

    pub fn matches_username(&self, username: &str) -> bool {
        if let Some(banned_username) = &self.username {
            banned_username.eq_ignore_ascii_case(username)
        } else {
            false
        }
    }

    pub fn time_until_expiry(&self) -> Option<Duration> {
        self.expires_at.map(|expires_at| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if now >= expires_at {
                Duration::from_secs(0)
            } else {
                Duration::from_secs(expires_at - now)
            }
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanAuditLogEntry {
    pub id: String,
    pub operation: BanOperation,
    pub ban_entry: BanEntry,
    pub timestamp: u64,
    pub performed_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BanOperation {
    Add,
    Remove,
    Update,
    Expire,
}

impl BanAuditLogEntry {
    pub fn new(operation: BanOperation, ban_entry: BanEntry, performed_by: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            id: Uuid::new_v4().to_string(),
            operation,
            ban_entry,
            timestamp: now,
            performed_by,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanConfig {
    pub storage_type: BanStorageType,

    pub file_path: Option<String>,

    pub audit_file_path: Option<String>,

    pub redis_url: Option<String>,

    pub database_url: Option<String>,

    pub enable_audit_log: bool,

    pub auto_cleanup_interval: u64,

    pub cache_size: usize,
}

impl Default for BanConfig {
    fn default() -> Self {
        Self {
            storage_type: BanStorageType::File,
            file_path: Some("bans.json".to_string()),
            audit_file_path: None, // Default to side-by-side file with .audit.json extension
            redis_url: None,
            database_url: None,
            enable_audit_log: true,
            auto_cleanup_interval: 3600, // 1 hour
            cache_size: 10_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BanStorageType {
    Memory,
    File,
    //TODO
    #[cfg(feature = "redis")]
    Redis,
    //TODO
    #[cfg(feature = "database")]
    Database,
}
#[derive(Debug)]
pub struct BanSystem {
    storage: BanStorage,
    config: BanConfig,
    auto_cleanup_handle: Option<tokio::task::JoinHandle<()>>,
}

impl BanSystem {
    pub async fn new(config: BanConfig) -> Result<Self, BanError> {
        let storage = BanStorage::new(&config).await?;

        let mut system = Self {
            storage,
            config,
            auto_cleanup_handle: None,
        };

        if system.config.auto_cleanup_interval > 0 {
            system.start_auto_cleanup();
        }

        Ok(system)
    }

    fn start_auto_cleanup(&mut self) {
        let interval = Duration::from_secs(self.config.auto_cleanup_interval);
        let storage = self.storage.clone();

        self.auto_cleanup_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);

            loop {
                interval.tick().await;

                match storage.clear_expired_bans().await {
                    Ok(count) => {
                        if count > 0 {
                            info!("Auto-cleanup removed {} expired bans", count);
                        } else {
                            debug!("Auto-cleanup found no expired bans");
                        }
                    }
                    Err(e) => {
                        error!("Error during auto-cleanup of expired bans: {}", e);
                    }
                }
            }
        }));
    }

    pub async fn add_ban(&self, ban: BanEntry) -> Result<(), BanError> {
        // Audit logging
        if self.config.enable_audit_log {
            let audit_entry =
                BanAuditLogEntry::new(BanOperation::Add, ban.clone(), ban.banned_by.clone());

            if let Err(e) = self.storage.add_audit_log(audit_entry).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to add audit log entry: {}", e
                );
            }
        }

        self.storage.add_ban(ban).await
    }

    pub async fn add_bans_batch(&self, bans: Vec<BanEntry>) -> Result<(), BanError> {
        if bans.is_empty() {
            return Ok(());
        }

        // Audit logging
        if self.config.enable_audit_log {
            let audit_entries = bans
                .iter()
                .map(|ban| {
                    BanAuditLogEntry::new(BanOperation::Add, ban.clone(), ban.banned_by.clone())
                })
                .collect::<Vec<_>>();

            if let Err(e) = self.storage.add_audit_logs_batch(audit_entries).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to add audit log entries in batch: {}", e
                );
            }
        }

        self.storage.add_bans_batch(bans).await
    }

    pub async fn remove_ban(&self, ban_id: &str, removed_by: &str) -> Result<BanEntry, BanError> {
        let ban = self.storage.get_ban_by_id(ban_id).await?;

        // Audit logging
        if self.config.enable_audit_log {
            let audit_entry =
                BanAuditLogEntry::new(BanOperation::Remove, ban.clone(), removed_by.to_string());

            if let Err(e) = self.storage.add_audit_log(audit_entry).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to add audit log entry: {}", e
                );
            }
        }

        self.storage.remove_ban(ban_id).await
    }

    pub async fn remove_ban_by_ip(
        &self,
        ip: &IpAddr,
        removed_by: &str,
    ) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.storage.get_bans_by_ip(ip).await?;

        if bans.is_empty() {
            return Err(BanError::NotFound);
        }

        // Audit logging
        if self.config.enable_audit_log {
            let audit_entries = bans
                .iter()
                .map(|ban| {
                    BanAuditLogEntry::new(BanOperation::Remove, ban.clone(), removed_by.to_string())
                })
                .collect::<Vec<_>>();

            if let Err(e) = self.storage.add_audit_logs_batch(audit_entries).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to add audit log entries in batch: {}", e
                );
            }
        }

        for ban in &bans {
            if let Err(e) = self.storage.remove_ban(&ban.id).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to remove ban {}: {}", ban.id, e
                );
            }
        }

        self.verify_ban_removal(Some(ip), None, None).await;
        Ok(bans)
    }

    pub async fn remove_ban_by_uuid(
        &self,
        uuid: &str,
        removed_by: &str,
    ) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.storage.get_bans_by_uuid(uuid).await?;

        if bans.is_empty() {
            return Err(BanError::NotFound);
        }

        // Audit logging
        if self.config.enable_audit_log {
            let audit_entries = bans
                .iter()
                .map(|ban| {
                    BanAuditLogEntry::new(BanOperation::Remove, ban.clone(), removed_by.to_string())
                })
                .collect::<Vec<_>>();

            if let Err(e) = self.storage.add_audit_logs_batch(audit_entries).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to add audit log entries in batch: {}", e
                );
            }
        }

        for ban in &bans {
            if let Err(e) = self.storage.remove_ban(&ban.id).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to remove ban {}: {}", ban.id, e
                );
            }
        }

        self.verify_ban_removal(None, Some(uuid), None).await;
        Ok(bans)
    }

    pub async fn remove_ban_by_username(
        &self,
        username: &str,
        removed_by: &str,
    ) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.storage.get_bans_by_username(username).await?;

        if bans.is_empty() {
            return Err(BanError::NotFound);
        }

        if self.config.enable_audit_log {
            let audit_entries = bans
                .iter()
                .map(|ban| {
                    BanAuditLogEntry::new(BanOperation::Remove, ban.clone(), removed_by.to_string())
                })
                .collect::<Vec<_>>();

            if let Err(e) = self.storage.add_audit_logs_batch(audit_entries).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to add audit log entries in batch: {}", e
                );
            }
        }

        for ban in &bans {
            if let Err(e) = self.storage.remove_ban(&ban.id).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to remove ban {}: {}", ban.id, e
                );
            }
        }

        self.verify_ban_removal(None, None, Some(username)).await;

        Ok(bans)
    }

    async fn verify_ban_removal(
        &self,
        ip: Option<&IpAddr>,
        uuid: Option<&str>,
        username: Option<&str>,
    ) {
        if let Some(ip_val) = ip {
            match self.storage.is_ip_banned(ip_val).await {
                Ok(false) => debug!(
                    log_type = "ban_system",
                    "Successfully verified IP {} is no longer banned", ip_val
                ),
                Ok(true) => warn!(
                    log_type = "ban_system",
                    "IP {} still appears as banned after removal!", ip_val
                ),
                Err(e) => warn!(
                    log_type = "ban_system",
                    "Failed to verify ban removal status for IP {}: {}", ip_val, e
                ),
            }
        }

        if let Some(uuid_val) = uuid {
            match self.storage.is_uuid_banned(uuid_val).await {
                Ok(false) => debug!(
                    log_type = "ban_system",
                    "Successfully verified UUID {} is no longer banned", uuid_val
                ),
                Ok(true) => warn!(
                    log_type = "ban_system",
                    "UUID {} still appears as banned after removal!", uuid_val
                ),
                Err(e) => warn!(
                    log_type = "ban_system",
                    "Failed to verify ban removal status for UUID {}: {}", uuid_val, e
                ),
            }
        }

        if let Some(username_val) = username {
            match self.storage.is_username_banned(username_val).await {
                Ok(false) => debug!(
                    log_type = "ban_system",
                    "Successfully verified username {} is no longer banned", username_val
                ),
                Ok(true) => warn!(
                    log_type = "ban_system",
                    "Username {} still appears as banned after removal!", username_val
                ),
                Err(e) => warn!(
                    log_type = "ban_system",
                    "Failed to verify ban removal status for username {}: {}", username_val, e
                ),
            }
        }
    }

    pub async fn is_ip_banned(&self, ip: &IpAddr) -> Result<bool, BanError> {
        self.storage.is_ip_banned(ip).await
    }

    pub async fn is_uuid_banned(&self, uuid: &str) -> Result<bool, BanError> {
        self.storage.is_uuid_banned(uuid).await
    }

    pub async fn is_username_banned(&self, username: &str) -> Result<bool, BanError> {
        self.storage.is_username_banned(username).await
    }

    pub async fn get_ban_reason_for_ip(&self, ip: &IpAddr) -> Result<Option<String>, BanError> {
        self.storage.get_ban_reason_for_ip(ip).await
    }

    pub async fn get_ban_reason_for_uuid(&self, uuid: &str) -> Result<Option<String>, BanError> {
        self.storage.get_ban_reason_for_uuid(uuid).await
    }

    pub async fn get_ban_reason_for_username(
        &self,
        username: &str,
    ) -> Result<Option<String>, BanError> {
        self.storage.get_ban_reason_for_username(username).await
    }

    pub async fn get_ban(&self, ban_id: &str) -> Result<BanEntry, BanError> {
        self.storage.get_ban_by_id(ban_id).await
    }

    pub async fn get_all_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        self.storage.get_all_bans().await
    }

    pub async fn get_active_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        self.storage.get_active_bans().await
    }

    pub async fn get_active_bans_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanEntry>, usize), BanError> {
        self.storage.get_active_bans_paged(page, page_size).await
    }

    pub async fn clear_expired_bans(&self) -> Result<usize, BanError> {
        let bans = self.storage.get_expired_bans().await?;

        if bans.is_empty() {
            return Ok(0);
        }

        if self.config.enable_audit_log {
            let audit_entries = bans
                .iter()
                .map(|ban| {
                    BanAuditLogEntry::new(BanOperation::Expire, ban.clone(), "system".to_string())
                })
                .collect::<Vec<_>>();

            if let Err(e) = self.storage.add_audit_logs_batch(audit_entries).await {
                warn!(
                    log_type = "ban_system",
                    "Failed to add audit log entries in batch: {}", e
                );
            }
        }

        self.storage.clear_expired_bans().await
    }

    pub async fn get_audit_logs(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanAuditLogEntry>, usize), BanError> {
        self.storage.get_audit_logs_paged(page, page_size).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn search_bans(
        &self,
        ip: Option<IpAddr>,
        uuid: Option<&str>,
        username: Option<&str>,
        reason_contains: Option<&str>,
        banned_by: Option<&str>,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanEntry>, usize), BanError> {
        self.storage
            .search_bans(
                ip,
                uuid,
                username,
                reason_contains,
                banned_by,
                page,
                page_size,
            )
            .await
    }

    pub async fn get_statistics(&self) -> Result<BanStatistics, BanError> {
        self.storage.get_statistics().await
    }
}

impl Drop for BanSystem {
    fn drop(&mut self) {
        if let Some(handle) = self.auto_cleanup_handle.take() {
            handle.abort();
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanStatistics {
    pub total_bans: usize,
    pub active_bans: usize,
    pub expired_bans: usize,
    pub permanent_bans: usize,
    pub temporary_bans: usize,
    pub ip_bans: usize,
    pub uuid_bans: usize,
    pub username_bans: usize,
}
