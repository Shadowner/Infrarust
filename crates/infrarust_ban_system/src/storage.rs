//! Storage trait and implementation for ban data

use std::sync::Arc;
use std::{fmt::Debug, net::IpAddr};

use async_trait::async_trait;
use tracing::{info, warn};

use crate::file::ban_storage::FileBanStorage;

use super::{
    BanAuditLogEntry, BanConfig, BanEntry, BanError, BanStatistics, BanStorageType,
    memory::MemoryBanStorage,
};

#[cfg(feature = "redis")]
use super::redis::RedisBanStorage;

#[cfg(feature = "database")]
use super::database::DatabaseBanStorage;

#[async_trait]
pub trait BanStorageBackend: Send + Sync + Debug {
    async fn add_ban(&self, ban: BanEntry) -> Result<(), BanError>;

    async fn add_bans_batch(&self, bans: Vec<BanEntry>) -> Result<(), BanError>;
    async fn remove_ban(&self, ban_id: &str) -> Result<BanEntry, BanError>;
    async fn get_ban_by_id(&self, ban_id: &str) -> Result<BanEntry, BanError>;
    async fn get_bans_by_ip(&self, ip: &IpAddr) -> Result<Vec<BanEntry>, BanError>;
    async fn get_bans_by_uuid(&self, uuid: &str) -> Result<Vec<BanEntry>, BanError>;
    async fn get_bans_by_username(&self, username: &str) -> Result<Vec<BanEntry>, BanError>;
    async fn is_ip_banned(&self, ip: &IpAddr) -> Result<bool, BanError>;
    async fn is_uuid_banned(&self, uuid: &str) -> Result<bool, BanError>;
    async fn is_username_banned(&self, username: &str) -> Result<bool, BanError>;
    async fn get_ban_reason_for_ip(&self, ip: &IpAddr) -> Result<Option<String>, BanError>;
    async fn get_ban_reason_for_uuid(&self, uuid: &str) -> Result<Option<String>, BanError>;
    async fn get_ban_reason_for_username(&self, username: &str)
    -> Result<Option<String>, BanError>;
    async fn get_all_bans(&self) -> Result<Vec<BanEntry>, BanError>;
    async fn get_active_bans(&self) -> Result<Vec<BanEntry>, BanError>;
    async fn get_active_bans_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanEntry>, usize), BanError>;
    async fn get_expired_bans(&self) -> Result<Vec<BanEntry>, BanError>;
    async fn clear_expired_bans(&self) -> Result<usize, BanError>;
    async fn add_audit_log(&self, entry: BanAuditLogEntry) -> Result<(), BanError>;
    async fn add_audit_logs_batch(&self, entries: Vec<BanAuditLogEntry>) -> Result<(), BanError>;
    async fn get_audit_logs_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanAuditLogEntry>, usize), BanError>;

    //TODO: Implement search_bans struct
    #[allow(clippy::too_many_arguments)]
    async fn search_bans(
        &self,
        ip: Option<IpAddr>,
        uuid: Option<&str>,
        username: Option<&str>,
        reason_contains: Option<&str>,
        banned_by: Option<&str>,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanEntry>, usize), BanError>;

    async fn get_statistics(&self) -> Result<BanStatistics, BanError>;
}

#[derive(Clone, Debug)]
pub struct BanStorage {
    backend: Arc<dyn BanStorageBackend>,
}

impl BanStorage {
    pub async fn new(config: &BanConfig) -> Result<Self, BanError> {
        let backend: Arc<dyn BanStorageBackend> = match config.storage_type {
            BanStorageType::Memory => {
                info!("Initializing in-memory ban storage");
                warn!("In-memory ban storage is not persistent and will be lost on restart");
                Arc::new(MemoryBanStorage::new())
            }
            BanStorageType::File => {
                let path = config
                    .file_path
                    .clone()
                    .unwrap_or_else(|| "bans.json".to_string());
                let audit_path = config.audit_file_path.clone();

                info!("Initializing file-based ban storage at {}", path);

                if let Some(audit_path_str) = &audit_path {
                    if audit_path_str.is_empty() {
                        info!(
                            "Audit logs will be stored alongside ban file with '.audit.json' extension"
                        );
                    } else {
                        info!("Audit logs will be stored at {}", audit_path_str);
                    }
                } else {
                    info!(
                        "Audit logs will be stored alongside ban file with '.audit.json' extension"
                    );
                }

                Arc::new(
                    FileBanStorage::new(&path, audit_path.as_deref(), config.cache_size).await?,
                )
            }
            //TODO
            #[cfg(feature = "redis")]
            BanStorageType::Redis => {
                let url = config.redis_url.clone().ok_or_else(|| {
                    BanError::Storage("Redis URL not provided in config".to_string())
                })?;
                info!("Initializing Redis ban storage");
                Arc::new(RedisBanStorage::new(&url, config.cache_size).await?)
            }
            //TODO
            #[cfg(feature = "database")]
            BanStorageType::Database => {
                let url = config.database_url.clone().ok_or_else(|| {
                    BanError::Storage("Database URL not provided in config".to_string())
                })?;
                info!("Initializing database ban storage");
                Arc::new(DatabaseBanStorage::new(&url, config.cache_size).await?)
            }
        };

        Ok(Self { backend })
    }
}

#[async_trait]
impl BanStorageBackend for BanStorage {
    async fn add_ban(&self, ban: BanEntry) -> Result<(), BanError> {
        self.backend.add_ban(ban).await
    }

    async fn add_bans_batch(&self, bans: Vec<BanEntry>) -> Result<(), BanError> {
        self.backend.add_bans_batch(bans).await
    }

    async fn remove_ban(&self, ban_id: &str) -> Result<BanEntry, BanError> {
        self.backend.remove_ban(ban_id).await
    }

    async fn get_ban_by_id(&self, ban_id: &str) -> Result<BanEntry, BanError> {
        self.backend.get_ban_by_id(ban_id).await
    }

    async fn get_bans_by_ip(&self, ip: &IpAddr) -> Result<Vec<BanEntry>, BanError> {
        self.backend.get_bans_by_ip(ip).await
    }

    async fn get_bans_by_uuid(&self, uuid: &str) -> Result<Vec<BanEntry>, BanError> {
        self.backend.get_bans_by_uuid(uuid).await
    }

    async fn get_bans_by_username(&self, username: &str) -> Result<Vec<BanEntry>, BanError> {
        self.backend.get_bans_by_username(username).await
    }

    async fn is_ip_banned(&self, ip: &IpAddr) -> Result<bool, BanError> {
        self.backend.is_ip_banned(ip).await
    }

    async fn is_uuid_banned(&self, uuid: &str) -> Result<bool, BanError> {
        self.backend.is_uuid_banned(uuid).await
    }

    async fn is_username_banned(&self, username: &str) -> Result<bool, BanError> {
        self.backend.is_username_banned(username).await
    }

    async fn get_ban_reason_for_ip(&self, ip: &IpAddr) -> Result<Option<String>, BanError> {
        self.backend.get_ban_reason_for_ip(ip).await
    }

    async fn get_ban_reason_for_uuid(&self, uuid: &str) -> Result<Option<String>, BanError> {
        self.backend.get_ban_reason_for_uuid(uuid).await
    }

    async fn get_ban_reason_for_username(
        &self,
        username: &str,
    ) -> Result<Option<String>, BanError> {
        self.backend.get_ban_reason_for_username(username).await
    }

    async fn get_all_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        self.backend.get_all_bans().await
    }

    async fn get_active_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        self.backend.get_active_bans().await
    }

    async fn get_active_bans_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanEntry>, usize), BanError> {
        self.backend.get_active_bans_paged(page, page_size).await
    }

    async fn get_expired_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        self.backend.get_expired_bans().await
    }

    async fn clear_expired_bans(&self) -> Result<usize, BanError> {
        self.backend.clear_expired_bans().await
    }

    async fn add_audit_log(&self, entry: BanAuditLogEntry) -> Result<(), BanError> {
        self.backend.add_audit_log(entry).await
    }

    async fn add_audit_logs_batch(&self, entries: Vec<BanAuditLogEntry>) -> Result<(), BanError> {
        self.backend.add_audit_logs_batch(entries).await
    }

    async fn get_audit_logs_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanAuditLogEntry>, usize), BanError> {
        self.backend.get_audit_logs_paged(page, page_size).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn search_bans(
        &self,
        ip: Option<IpAddr>,
        uuid: Option<&str>,
        username: Option<&str>,
        reason_contains: Option<&str>,
        banned_by: Option<&str>,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanEntry>, usize), BanError> {
        self.backend
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

    async fn get_statistics(&self) -> Result<BanStatistics, BanError> {
        self.backend.get_statistics().await
    }
}
