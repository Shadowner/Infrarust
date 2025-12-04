//! Efficient indexing for ban lookups

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use tokio::sync::RwLock;

use super::BanEntry;

/// Thread-safe indexing structure for fast ban lookups
#[derive(Clone, Debug)]
pub struct BanIndex {
    by_id: Arc<DashMap<String, Arc<BanEntry>>>,
    by_ip: Arc<DashMap<IpAddr, HashSet<String>>>,
    by_uuid: Arc<DashMap<String, HashSet<String>>>,
    by_username: Arc<DashMap<String, HashSet<String>>>,
    by_banned_by: Arc<DashMap<String, HashSet<String>>>,

    // For range queries (expiration time)
    by_expiry: Arc<RwLock<HashMap<Option<u64>, HashSet<String>>>>,
}

impl Default for BanIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl BanIndex {
    pub fn new() -> Self {
        Self {
            by_id: Arc::new(DashMap::new()),
            by_ip: Arc::new(DashMap::new()),
            by_uuid: Arc::new(DashMap::new()),
            by_username: Arc::new(DashMap::new()),
            by_banned_by: Arc::new(DashMap::new()),
            by_expiry: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add(&self, ban: Arc<BanEntry>) {
        self.by_id.insert(ban.id.clone(), ban.clone());

        if let Some(ip) = ban.ip {
            self.by_ip.entry(ip).or_default().insert(ban.id.clone());
        }

        if let Some(uuid) = &ban.uuid {
            self.by_uuid
                .entry(uuid.clone())
                .or_default()
                .insert(ban.id.clone());
        }

        if let Some(username) = &ban.username {
            let username_lower = username.to_lowercase();
            self.by_username
                .entry(username_lower)
                .or_default()
                .insert(ban.id.clone());
        }

        self.by_banned_by
            .entry(ban.banned_by.clone())
            .or_default()
            .insert(ban.id.clone());

        let mut expiry_index = self.by_expiry.write().await;
        expiry_index
            .entry(ban.expires_at)
            .or_insert_with(HashSet::new)
            .insert(ban.id.clone());
    }

    pub async fn remove(&self, ban_id: &str) -> Option<Arc<BanEntry>> {
        let ban = self.by_id.remove(ban_id)?;

        let remove_from_index = |map: &DashMap<String, HashSet<String>>, key: &str| {
            if let Some(mut entry) = map.get_mut(key) {
                entry.remove(ban_id);
                if entry.is_empty() {
                    drop(entry);
                    map.remove(key);
                }
            }
        };

        // Remove from IP index
        if let Some(ip) = ban.1.ip
            && let Some(mut entry) = self.by_ip.get_mut(&ip)
        {
            entry.remove(ban_id);
            if entry.is_empty() {
                drop(entry);
                self.by_ip.remove(&ip);
            }
        }

        // Remove from UUID index
        if let Some(uuid) = &ban.1.uuid {
            remove_from_index(&self.by_uuid, uuid);
        }

        // Remove from username index
        if let Some(username) = &ban.1.username {
            let username_lower = username.to_lowercase();
            remove_from_index(&self.by_username, &username_lower);
        }

        // Remove from banned_by index
        remove_from_index(&self.by_banned_by, &ban.1.banned_by);

        // Remove from expiry index
        let mut expiry_index = self.by_expiry.write().await;
        if let Some(entry) = expiry_index.get_mut(&ban.1.expires_at) {
            entry.remove(ban_id);
            if entry.is_empty() {
                expiry_index.remove(&ban.1.expires_at);
            }
        }

        Some(ban.1)
    }

    pub fn get_by_id(&self, ban_id: &str) -> Option<Arc<BanEntry>> {
        self.by_id.get(ban_id).map(|v| v.clone())
    }

    fn collect_bans_from_index(&self, ids: &HashSet<String>) -> Vec<Arc<BanEntry>> {
        ids.iter()
            .filter_map(|id| self.by_id.get(id).map(|v| v.clone()))
            .collect()
    }

    pub fn get_by_ip(&self, ip: &IpAddr) -> Vec<Arc<BanEntry>> {
        match self.by_ip.get(ip) {
            Some(ids) => self.collect_bans_from_index(&ids),
            None => Vec::new(),
        }
    }

    pub fn get_by_uuid(&self, uuid: &str) -> Vec<Arc<BanEntry>> {
        match self.by_uuid.get(uuid) {
            Some(ids) => self.collect_bans_from_index(&ids),
            None => Vec::new(),
        }
    }

    pub fn get_by_username(&self, username: &str) -> Vec<Arc<BanEntry>> {
        let username_lower = username.to_lowercase();
        match self.by_username.get(&username_lower) {
            Some(ids) => self.collect_bans_from_index(&ids),
            None => Vec::new(),
        }
    }

    pub fn get_by_banned_by(&self, banned_by: &str) -> Vec<Arc<BanEntry>> {
        match self.by_banned_by.get(banned_by) {
            Some(ids) => self.collect_bans_from_index(&ids),
            None => Vec::new(),
        }
    }

    fn get_current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub async fn get_expired(&self) -> Vec<Arc<BanEntry>> {
        let now = Self::get_current_timestamp();
        let expiry_index = self.by_expiry.read().await;
        let mut result = Vec::new();

        for (expires_at, ids) in expiry_index.iter() {
            if let Some(exp) = expires_at
                && *exp <= now
            {
                for id in ids {
                    if let Some(ban) = self.by_id.get(id) {
                        result.push(ban.clone());
                    }
                }
            }
        }

        result
    }

    pub async fn get_active(&self) -> Vec<Arc<BanEntry>> {
        let now = Self::get_current_timestamp();
        let expiry_index = self.by_expiry.read().await;
        let mut result = Vec::new();

        for (expires_at, ids) in expiry_index.iter() {
            let is_active = match expires_at {
                None => true, // Permanent bans are always active
                Some(exp) => *exp > now,
            };

            if is_active {
                for id in ids {
                    if let Some(ban) = self.by_id.get(id) {
                        result.push(ban.clone());
                    }
                }
            }
        }

        result
    }

    pub fn get_all(&self) -> Vec<Arc<BanEntry>> {
        self.by_id.iter().map(|e| e.value().clone()).collect()
    }

    pub async fn search(
        &self,
        ip: Option<IpAddr>,
        uuid: Option<&str>,
        username: Option<&str>,
        reason_contains: Option<&str>,
        banned_by: Option<&str>,
    ) -> Vec<Arc<BanEntry>> {
        let mut candidate_ids: Option<HashSet<String>> = None;

        if let Some(ip_val) = ip {
            if let Some(ids) = self.by_ip.get(&ip_val) {
                candidate_ids = Some(ids.clone());
            } else {
                return Vec::new(); // No matches for IP
            }
        } else if let Some(uuid_val) = uuid {
            if let Some(ids) = self.by_uuid.get(uuid_val) {
                candidate_ids = Some(ids.clone());
            } else {
                return Vec::new(); // No matches for UUID
            }
        } else if let Some(username_val) = username {
            let username_lower = username_val.to_lowercase();
            if let Some(ids) = self.by_username.get(&username_lower) {
                candidate_ids = Some(ids.clone());
            } else {
                return Vec::new(); // No matches for username
            }
        } else if let Some(banned_by_val) = banned_by {
            if let Some(ids) = self.by_banned_by.get(banned_by_val) {
                candidate_ids = Some(ids.clone());
            } else {
                return Vec::new(); // No matches for banned_by
            }
        }

        let mut result = if let Some(ids) = candidate_ids {
            self.collect_bans_from_index(&ids)
        } else {
            self.get_all() // No specific criteria, start with all bans
        };

        self.apply_search_filters(&mut result, ip, uuid, username, reason_contains, banned_by);

        result
    }

    fn apply_search_filters(
        &self,
        result: &mut Vec<Arc<BanEntry>>,
        ip: Option<IpAddr>,
        uuid: Option<&str>,
        username: Option<&str>,
        reason_contains: Option<&str>,
        banned_by: Option<&str>,
    ) {
        if let Some(ip_val) = ip {
            result.retain(|ban| ban.ip == Some(ip_val));
        }

        if let Some(uuid_val) = uuid {
            result.retain(|ban| {
                ban.uuid
                    .as_ref()
                    .is_some_and(|ban_uuid| ban_uuid == uuid_val)
            });
        }

        if let Some(username_val) = username {
            result.retain(|ban| {
                ban.username
                    .as_ref()
                    .is_some_and(|ban_username| ban_username.eq_ignore_ascii_case(username_val))
            });
        }

        if let Some(reason_val) = reason_contains {
            let reason_lower = reason_val.to_lowercase();
            result.retain(|ban| ban.reason.to_lowercase().contains(&reason_lower));
        }

        if let Some(banned_by_val) = banned_by {
            result.retain(|ban| ban.banned_by == banned_by_val);
        }
    }

    fn is_ban_active(ban: &BanEntry) -> bool {
        match ban.expires_at {
            None => true, // Permanent ban
            Some(expires_at) => expires_at > Self::get_current_timestamp(),
        }
    }

    pub async fn is_ip_banned(&self, ip: &IpAddr) -> bool {
        if !self.by_ip.contains_key(ip) {
            return false;
        }

        let bans = self.get_by_ip(ip);
        bans.iter().any(|ban| Self::is_ban_active(ban))
    }

    pub async fn is_uuid_banned(&self, uuid: &str) -> bool {
        if !self.by_uuid.contains_key(uuid) {
            return false;
        }

        let bans = self.get_by_uuid(uuid);
        bans.iter().any(|ban| Self::is_ban_active(ban))
    }

    pub async fn is_username_banned(&self, username: &str) -> bool {
        let username_lower = username.to_lowercase();
        if !self.by_username.contains_key(&username_lower) {
            return false;
        }

        let bans = self.get_by_username(username);
        bans.iter().any(|ban| Self::is_ban_active(ban))
    }

    pub fn count(&self) -> usize {
        self.by_id.len()
    }
}
