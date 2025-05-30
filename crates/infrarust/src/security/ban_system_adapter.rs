/// Why: I did this adapter to separate the ban system and in the future
/// make it easier to refacto with a multi-crates project.
/// This adapter is a bridge between the new ban system and the Filter trait.
use std::{any::Any, io, net::IpAddr, path::Path, sync::Arc};

use async_trait::async_trait;
use tokio::net::TcpStream;
use tracing::{debug, error, warn};

use crate::security::filter::{ConfigValue, Filter, FilterError, FilterType};

use infrarust_ban_system::{BanConfig, BanEntry, BanError, BanStorageType, BanSystem};
use infrarust_config::LogType;

#[derive(Debug)]
pub struct BanSystemAdapter {
    name: String,
    ban_system: Arc<BanSystem>,
}

impl BanSystemAdapter {
    pub async fn new(
        name: impl Into<String>,
        file_path: impl Into<String>,
    ) -> Result<Self, FilterError> {
        let name = name.into();
        let file_path_str = file_path.into();

        debug!(
            log_type = LogType::BanSystem.as_str(),
            "Creating BanSystemAdapter with file storage: {}",
            file_path_str
        );

        let path_obj = Path::new(&file_path_str);
        if let Some(parent) = path_obj.parent() {
            if !parent.exists() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    error!(log_type = LogType::BanSystem.as_str(), "Failed to create ban directory {}: {}", parent.display(), e);
                    return Err(FilterError::IoError(e));
                } else {
                    debug!(log_type = LogType::BanSystem.as_str(), "Created ban directory: {}", parent.display());
                }
            }
        }

        let config = BanConfig {
            storage_type: BanStorageType::File,
            file_path: Some(file_path_str.clone()),
            enable_audit_log: true,
            auto_cleanup_interval: 3600, // 1 hour
            cache_size: 10_000,
            ..Default::default()
        };

        let ban_system = match BanSystem::new(config).await {
            Ok(system) => Arc::new(system),
            Err(e) => {
                error!(log_type = LogType::BanSystem.as_str(), "Failed to initialize ban system: {}", e);
                return Err(FilterError::Other(format!(
                    "Failed to initialize ban system: {}",
                    e
                )));
            }
        };

        Ok(Self { name, ban_system })
    }

    pub async fn remove_ban_by_ip(
        &self,
        ip: &IpAddr,
        removed_by: &str,
    ) -> Result<bool, FilterError> {
        debug!(log_type = LogType::BanSystem.as_str(), "Attempting to remove ban for IP: {}", ip);
        match self.ban_system.remove_ban_by_ip(ip, removed_by).await {
            Ok(bans) => {
                debug!(log_type = LogType::BanSystem.as_str(), "Successfully removed {} bans for IP: {}", bans.len(), ip);
                // Force a refresh to ensure all cache entries are cleared
                let _ = self.refresh().await;
                Ok(!bans.is_empty())
            }
            Err(BanError::NotFound) => Ok(false),
            Err(e) => Err(FilterError::Other(format!("Failed to remove ban: {}", e))),
        }
    }

    pub async fn remove_ban_by_uuid(
        &self,
        uuid: &str,
        removed_by: &str,
    ) -> Result<bool, FilterError> {
        debug!(log_type = LogType::BanSystem.as_str(), "Attempting to remove ban for UUID: {}", uuid);
        match self.ban_system.remove_ban_by_uuid(uuid, removed_by).await {
            Ok(bans) => {
                debug!(
                    log_type = LogType::BanSystem.as_str(),
                    "Successfully removed {} bans for UUID: {}",
                    bans.len(),
                    uuid
                );
                // Force a refresh to ensure all cache entries are cleared
                let _ = self.refresh().await;
                Ok(!bans.is_empty())
            }
            Err(BanError::NotFound) => Ok(false),
            Err(e) => Err(FilterError::Other(format!("Failed to remove ban: {}", e))),
        }
    }

    pub async fn remove_ban_by_username(
        &self,
        username: &str,
        removed_by: &str,
    ) -> Result<bool, FilterError> {
        debug!(log_type = LogType::BanSystem.as_str(), "Attempting to remove ban for username: {}", username);
        match self
            .ban_system
            .remove_ban_by_username(username, removed_by)
            .await
        {
            Ok(bans) => {
                debug!(
                    log_type = LogType::BanSystem.as_str(),
                    "Successfully removed {} bans for username: {}",
                    bans.len(),
                    username
                );
                // Force a refresh to ensure all cache entries are cleared
                let _ = self.refresh().await;
                Ok(!bans.is_empty())
            }
            Err(BanError::NotFound) => Ok(false),
            Err(e) => Err(FilterError::Other(format!("Failed to remove ban: {}", e))),
        }
    }

    pub async fn clear_expired_bans(&self) -> Result<usize, FilterError> {
        match self.ban_system.clear_expired_bans().await {
            Ok(count) => Ok(count),
            Err(e) => Err(FilterError::Other(format!(
                "Failed to clear expired bans: {}",
                e
            ))),
        }
    }

    pub async fn is_ip_banned(&self, ip: &IpAddr) -> Result<bool, FilterError> {
        match self.ban_system.is_ip_banned(ip).await {
            Ok(banned) => Ok(banned),
            Err(e) => Err(FilterError::Other(format!(
                "Failed to check if IP is banned: {}",
                e
            ))),
        }
    }

    pub async fn is_uuid_banned(&self, uuid: &str) -> Result<bool, FilterError> {
        match self.ban_system.is_uuid_banned(uuid).await {
            Ok(banned) => Ok(banned),
            Err(e) => Err(FilterError::Other(format!(
                "Failed to check if UUID is banned: {}",
                e
            ))),
        }
    }

    pub async fn get_ban_reason_for_ip(&self, ip: &IpAddr) -> Result<Option<String>, FilterError> {
        match self.ban_system.get_ban_reason_for_ip(ip).await {
            Ok(reason) => Ok(reason),
            Err(e) => Err(FilterError::Other(format!(
                "Failed to get ban reason: {}",
                e
            ))),
        }
    }

    pub async fn is_username_banned(&self, username: &str) -> Result<bool, FilterError> {
        match self.ban_system.is_username_banned(username).await {
            Ok(banned) => Ok(banned),
            Err(e) => Err(FilterError::Other(format!(
                "Failed to check if username is banned: {}",
                e
            ))),
        }
    }

    pub async fn add_ban(&self, ban: BanEntry) -> Result<(), FilterError> {
        match self.ban_system.add_ban(ban).await {
            Ok(_) => Ok(()),
            Err(e) => Err(FilterError::Other(format!("Failed to add ban: {}", e))),
        }
    }

    pub async fn get_all_bans(&self) -> Result<Vec<BanEntry>, FilterError> {
        match self.ban_system.get_active_bans().await {
            Ok(bans) => Ok(bans),
            Err(e) => Err(FilterError::Other(format!(
                "Failed to get active bans: {}",
                e
            ))),
        }
    }

    pub async fn get_ban_reason_for_username(
        &self,
        username: &str,
    ) -> Result<Option<String>, FilterError> {
        match self.ban_system.get_ban_reason_for_username(username).await {
            Ok(reason) => Ok(reason),
            Err(e) => Err(FilterError::Other(format!(
                "Failed to get ban reason: {}",
                e
            ))),
        }
    }

    async fn refresh(&self) -> Result<(), FilterError> {
        debug!(log_type = LogType::BanSystem.as_str(), "Refreshing ban system");
        match self.ban_system.clear_expired_bans().await {
            Ok(_) => Ok(()),
            Err(e) => Err(FilterError::Other(format!("Failed to refresh bans: {}", e))),
        }
    }
}

#[async_trait]
impl Filter for BanSystemAdapter {
    async fn filter(&self, stream: &TcpStream) -> io::Result<()> {
        if let Ok(addr) = stream.peer_addr() {
            let ip = addr.ip();

            match self.ban_system.is_ip_banned(&ip).await {
                Ok(true) => {
                    let reason = match self.ban_system.get_ban_reason_for_ip(&ip).await {
                        Ok(Some(reason)) => reason,
                        _ => "Banned".to_string(),
                    };

                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("IP banned: {}", reason),
                    ));
                }
                Ok(false) => {}
                Err(e) => {
                    warn!(log_type = LogType::BanSystem.as_str(), "Error checking ban status: {}", e);
                    // We want to continue processing even if there's an error
                }
            }
        }

        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn filter_type(&self) -> FilterType {
        FilterType::BanFilter
    }

    fn is_configurable(&self) -> bool {
        true
    }

    async fn apply_config(&self, config: ConfigValue) -> Result<(), FilterError> {
        if let ConfigValue::Map(map) = config {
            if let Some(ConfigValue::String(_)) = map.get("storage_path") {
                // The ban system can't change its storage path
                // dynamically, so we'll just log a warning for now
                warn!(
                    "Storage path changes not supported in the new ban system. Restart required."
                );
                return Ok(());
            }

            return Err(FilterError::InvalidConfig(
                "Invalid configuration for BanFilter".to_string(),
            ));
        }

        Err(FilterError::InvalidConfig(
            "Expected a map configuration".to_string(),
        ))
    }

    fn is_refreshable(&self) -> bool {
        true
    }

    async fn refresh(&self) -> Result<(), FilterError> {
        debug!(log_type = LogType::BanSystem.as_str(), "Refreshing ban system");
        match self.ban_system.clear_expired_bans().await {
            Ok(_) => Ok(()),
            Err(e) => Err(FilterError::Other(format!("Failed to refresh bans: {}", e))),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
