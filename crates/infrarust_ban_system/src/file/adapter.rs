use std::{net::IpAddr, sync::Arc};

use crate::{BanAuditLogEntry, BanEntry, BanError, BanStatistics, BanStorageBackend};
use async_trait::async_trait;
use tracing::{debug, warn};

use super::ban_storage::FileBanStorage;

#[async_trait]
impl BanStorageBackend for FileBanStorage {
    async fn add_ban(&self, ban: BanEntry) -> Result<(), BanError> {
        debug!("Adding ban: {:?}", ban);
        self.index.add(Arc::new(ban)).await;
        self.mark_bans_dirty().await;
        Ok(())
    }

    async fn add_bans_batch(&self, bans: Vec<BanEntry>) -> Result<(), BanError> {
        debug!("Adding {} bans in batch", bans.len());
        for ban in bans {
            self.index.add(Arc::new(ban)).await;
        }
        self.mark_bans_dirty().await;
        Ok(())
    }

    async fn remove_ban(&self, ban_id: &str) -> Result<BanEntry, BanError> {
        debug!("Removing ban with ID: {}", ban_id);
        match self.index.remove(ban_id).await {
            Some(ban) => {
                debug!("Ban removed: {:?}", ban);
                self.mark_bans_dirty().await;

                // Verify ban was completely removed from indexes
                if let Some(ip) = ban.ip {
                    let still_banned = self.index.is_ip_banned(&ip).await;
                    if still_banned {
                        warn!("IP {} still appears banned after removal!", ip);
                    }
                }

                if let Some(uuid) = &ban.uuid {
                    let still_banned = self.index.is_uuid_banned(uuid).await;
                    if still_banned {
                        warn!("UUID {} still appears banned after removal!", uuid);
                    }
                }

                if let Some(username) = &ban.username {
                    let still_banned = self.index.is_username_banned(username).await;
                    if still_banned {
                        warn!("Username {} still appears banned after removal!", username);
                    }
                }

                Ok((*ban).clone())
            }
            None => {
                debug!("Ban not found with ID: {}", ban_id);
                Err(BanError::NotFound)
            }
        }
    }

    async fn get_ban_by_id(&self, ban_id: &str) -> Result<BanEntry, BanError> {
        match self.index.get_by_id(ban_id) {
            Some(ban) => Ok((*ban).clone()),
            None => Err(BanError::NotFound),
        }
    }

    async fn get_bans_by_ip(&self, ip: &IpAddr) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.index.get_by_ip(ip);
        Ok(bans.into_iter().map(|b| (*b).clone()).collect())
    }

    async fn get_bans_by_uuid(&self, uuid: &str) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.index.get_by_uuid(uuid);
        Ok(bans.into_iter().map(|b| (*b).clone()).collect())
    }

    async fn get_bans_by_username(&self, username: &str) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.index.get_by_username(username);
        Ok(bans.into_iter().map(|b| (*b).clone()).collect())
    }

    async fn is_ip_banned(&self, ip: &IpAddr) -> Result<bool, BanError> {
        Ok(self.index.is_ip_banned(ip).await)
    }

    async fn is_uuid_banned(&self, uuid: &str) -> Result<bool, BanError> {
        Ok(self.index.is_uuid_banned(uuid).await)
    }

    async fn is_username_banned(&self, username: &str) -> Result<bool, BanError> {
        Ok(self.index.is_username_banned(username).await)
    }

    async fn get_ban_reason_for_ip(&self, ip: &IpAddr) -> Result<Option<String>, BanError> {
        let bans = self.index.get_by_ip(ip);

        if bans.is_empty() {
            return Ok(None);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for ban in bans {
            if let Some(expires_at) = ban.expires_at {
                if expires_at > now {
                    return Ok(Some(ban.reason.clone()));
                }
            } else {
                // Permanent ban
                return Ok(Some(ban.reason.clone()));
            }
        }

        Ok(None)
    }

    async fn get_ban_reason_for_uuid(&self, uuid: &str) -> Result<Option<String>, BanError> {
        let bans = self.index.get_by_uuid(uuid);

        if bans.is_empty() {
            return Ok(None);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for ban in bans {
            if let Some(expires_at) = ban.expires_at {
                if expires_at > now {
                    return Ok(Some(ban.reason.clone()));
                }
            } else {
                // Permanent ban
                return Ok(Some(ban.reason.clone()));
            }
        }

        Ok(None)
    }

    async fn get_ban_reason_for_username(
        &self,
        username: &str,
    ) -> Result<Option<String>, BanError> {
        let bans = self.index.get_by_username(username);

        if bans.is_empty() {
            return Ok(None);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for ban in bans {
            if let Some(expires_at) = ban.expires_at {
                if expires_at > now {
                    return Ok(Some(ban.reason.clone()));
                }
            } else {
                // Permanent ban
                return Ok(Some(ban.reason.clone()));
            }
        }

        Ok(None)
    }

    async fn get_all_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.index.get_all();
        Ok(bans.into_iter().map(|b| (*b).clone()).collect())
    }

    async fn get_active_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.index.get_active().await;
        Ok(bans.into_iter().map(|b| (*b).clone()).collect())
    }

    async fn get_active_bans_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanEntry>, usize), BanError> {
        let all_bans = self.index.get_active().await;
        let total = all_bans.len();

        let start = page * page_size;
        let end = (start + page_size).min(total);

        if start >= total {
            return Ok((Vec::new(), total));
        }

        let paged_bans = all_bans[start..end].iter().map(|b| (**b).clone()).collect();

        Ok((paged_bans, total))
    }

    async fn get_expired_bans(&self) -> Result<Vec<BanEntry>, BanError> {
        let bans = self.index.get_expired().await;
        Ok(bans.into_iter().map(|b| (*b).clone()).collect())
    }

    async fn clear_expired_bans(&self) -> Result<usize, BanError> {
        let expired = self.index.get_expired().await;
        let count = expired.len();

        for ban in expired {
            let _ = self.index.remove(&ban.id).await;
        }

        if count > 0 {
            self.mark_bans_dirty().await;
            debug!("Cleared {} expired bans", count);
        }

        Ok(count)
    }

    async fn add_audit_log(&self, entry: BanAuditLogEntry) -> Result<(), BanError> {
        self.append_audit_log(entry).await
    }

    async fn add_audit_logs_batch(&self, entries: Vec<BanAuditLogEntry>) -> Result<(), BanError> {
        self.append_audit_logs_batch(entries).await
    }

    async fn get_audit_logs_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanAuditLogEntry>, usize), BanError> {
        self.load_audit_logs_paged(page, page_size).await
    }

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
        let results = self
            .index
            .search(ip, uuid, username, reason_contains, banned_by)
            .await;

        let total = results.len();

        let start = page * page_size;
        let end = (start + page_size).min(total);

        if start >= total {
            return Ok((Vec::new(), total));
        }

        let paged_results = results[start..end].iter().map(|b| (**b).clone()).collect();

        Ok((paged_results, total))
    }

    async fn get_statistics(&self) -> Result<BanStatistics, BanError> {
        let all_bans = self.index.get_all();
        let active_bans = self.index.get_active().await;
        let expired_bans = self.index.get_expired().await;

        let mut ip_bans = 0;
        let mut uuid_bans = 0;
        let mut username_bans = 0;
        let mut permanent_bans = 0;
        let mut temporary_bans = 0;

        for ban in &all_bans {
            if ban.ip.is_some() {
                ip_bans += 1;
            }

            if ban.uuid.is_some() {
                uuid_bans += 1;
            }

            if ban.username.is_some() {
                username_bans += 1;
            }

            if ban.expires_at.is_none() {
                permanent_bans += 1;
            } else {
                temporary_bans += 1;
            }
        }

        Ok(BanStatistics {
            total_bans: all_bans.len(),
            active_bans: active_bans.len(),
            expired_bans: expired_bans.len(),
            permanent_bans,
            temporary_bans,
            ip_bans,
            uuid_bans,
            username_bans,
        })
    }
}
