use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::IpAddr;
use std::ops::Bound;
use std::path::PathBuf;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use infrarust_api::services::ban_service::{
    BanPage, BanQuery, IpNet, LoginAttempt, LoginStage, epoch_serde, option_epoch_serde,
};

use crate::ban::storage::{BanStorage, StorageFuture};
use crate::ban::types::{BanAction, BanAuditLogEntry, BanEntry, BanSource, BanTarget};
use crate::error::CoreError;

const MAX_AUDIT_LOG_ENTRIES: usize = 10_000;

#[derive(Serialize, Deserialize, Default)]
struct BanFileData {
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    bans: Vec<StoredBan>,
    #[serde(default)]
    audit_log: Vec<BanAuditLogEntry>,
}

#[derive(Serialize, Deserialize)]
struct StoredBan {
    #[serde(default)]
    id: Option<String>,
    target: BanTarget,
    reason: Option<String>,
    #[serde(with = "option_epoch_serde")]
    expires_at: Option<SystemTime>,
    #[serde(with = "epoch_serde")]
    created_at: SystemTime,
    #[serde(deserialize_with = "stored_source")]
    source: BanSource,
}

impl StoredBan {
    fn from_entry(entry: &BanEntry) -> Self {
        Self {
            id: Some(entry.id.clone()),
            target: entry.target.clone(),
            reason: entry.reason.clone(),
            expires_at: entry.expires_at,
            created_at: entry.created_at,
            source: entry.source.clone(),
        }
    }

    fn into_entry(self, id: u64) -> BanEntry {
        let mut entry = BanEntry::new(id.to_string(), self.target.canonical(), self.source)
            .created_at(self.created_at);
        entry.reason = self.reason;
        entry.expires_at = self.expires_at;
        entry
    }
}

fn legacy_source(source: &str) -> BanSource {
    match source {
        "console" => BanSource::Console,
        "" | "system" => BanSource::System,
        "admin_api" | "web_api" | "web-api" => BanSource::WebApi { actor: None },
        other => BanSource::Plugin(other.to_string()),
    }
}

fn stored_source<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BanSource, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Current(BanSource),
        Legacy(String),
    }
    Ok(match Stored::deserialize(deserializer)? {
        Stored::Current(source) => source,
        Stored::Legacy(source) => legacy_source(&source),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum BanKey {
    Ip(IpAddr),
    Range(IpNet),
    Username(String),
    Uuid(Uuid),
}

fn key_of(target: &BanTarget) -> Option<BanKey> {
    match target.clone().canonical() {
        BanTarget::Ip(ip) => Some(BanKey::Ip(ip)),
        BanTarget::IpRange(net) => Some(BanKey::Range(net)),
        BanTarget::Username(name) => Some(BanKey::Username(name.to_lowercase())),
        BanTarget::Uuid(uuid) => Some(BanKey::Uuid(uuid)),
        _ => None,
    }
}

#[derive(Default)]
struct BanTable {
    by_id: BTreeMap<u64, BanEntry>,
    by_key: HashMap<BanKey, u64>,
    ranges: BTreeSet<u64>,
    last_id: u64,
}

impl BanTable {
    fn insert(&mut self, key: BanKey, id: u64, entry: BanEntry) {
        if let Some(previous) = self.by_key.insert(key, id) {
            self.by_id.remove(&previous);
            self.ranges.remove(&previous);
        }
        if matches!(entry.target, BanTarget::IpRange(_)) {
            self.ranges.insert(id);
        }
        self.last_id = self.last_id.max(id);
        self.by_id.insert(id, entry);
    }

    fn remove(&mut self, key: &BanKey) -> Option<BanEntry> {
        let id = self.by_key.remove(key)?;
        self.ranges.remove(&id);
        self.by_id.remove(&id)
    }

    fn active(&self, key: &BanKey) -> Option<&BanEntry> {
        self.by_key
            .get(key)
            .and_then(|id| self.by_id.get(id))
            .filter(|entry| !entry.is_expired())
    }

    fn check(&self, attempt: &LoginAttempt) -> Option<BanEntry> {
        let ip = attempt.ip.to_canonical();
        if let Some(entry) = self.active(&BanKey::Ip(ip)) {
            return Some(entry.clone());
        }
        let in_range = self
            .ranges
            .iter()
            .filter_map(|id| self.by_id.get(id))
            .find(|entry| !entry.is_expired() && entry.target.matches_ip(ip));
        if let Some(entry) = in_range {
            return Some(entry.clone());
        }
        if let Some(name) = attempt.username.as_deref()
            && let Some(entry) = self.active(&BanKey::Username(name.to_lowercase()))
        {
            return Some(entry.clone());
        }
        if attempt.stage == LoginStage::PostAuth
            && let Some(uuid) = attempt.uuid
            && let Some(entry) = self.active(&BanKey::Uuid(uuid))
        {
            return Some(entry.clone());
        }
        None
    }

    fn page(&self, after: Option<u64>, limit: usize) -> BanPage {
        let start = after.map_or(Bound::Unbounded, Bound::Excluded);
        let mut entries: Vec<BanEntry> = Vec::new();
        let mut next_cursor = None;
        for entry in self
            .by_id
            .range((start, Bound::Unbounded))
            .map(|(_, entry)| entry)
            .filter(|entry| !entry.is_expired())
        {
            if entries.len() == limit {
                next_cursor = entries.last().map(|last| last.id.clone());
                break;
            }
            entries.push(entry.clone());
        }
        BanPage::new(entries, next_cursor)
    }

    fn remove_expired(&mut self) -> Vec<BanEntry> {
        let expired: Vec<BanKey> = self
            .by_key
            .iter()
            .filter(|(_, id)| self.by_id.get(id).is_some_and(BanEntry::is_expired))
            .map(|(key, _)| key.clone())
            .collect();
        expired.iter().filter_map(|key| self.remove(key)).collect()
    }
}

pub struct FileBanStorage {
    table: RwLock<BanTable>,
    file_path: PathBuf,
    audit_log: tokio::sync::RwLock<Vec<BanAuditLogEntry>>,
    write_lock: tokio::sync::Mutex<()>,
}

impl FileBanStorage {
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            table: RwLock::new(BanTable::default()),
            file_path,
            audit_log: tokio::sync::RwLock::new(Vec::new()),
            write_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, BanTable> {
        self.table.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, BanTable> {
        self.table.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn serialize_all(&self, audit_log: &[BanAuditLogEntry]) -> Result<String, CoreError> {
        let (next_id, bans) = {
            let table = self.read();
            let bans: Vec<StoredBan> = table.by_id.values().map(StoredBan::from_entry).collect();
            (table.last_id, bans)
        };
        let data = BanFileData {
            next_id,
            bans,
            audit_log: audit_log.to_vec(),
        };
        serde_json::to_string_pretty(&data).map_err(|e| CoreError::Other(e.to_string()))
    }

    fn populate(&self, data: BanFileData) {
        let mut table = self.write();
        *table = BanTable {
            last_id: data.next_id,
            ..BanTable::default()
        };
        let mut unnumbered = Vec::new();
        for stored in data.bans {
            let id = stored
                .id
                .as_deref()
                .and_then(|id| id.parse::<u64>().ok())
                .filter(|id| *id > 0 && !table.by_id.contains_key(id));
            match id {
                Some(id) => restore(&mut table, stored, id),
                None => unnumbered.push(stored),
            }
        }
        for stored in unnumbered {
            let id = table.last_id + 1;
            restore(&mut table, stored, id);
        }
    }

    async fn add_audit_entry(&self, entry: BanAuditLogEntry) {
        let mut log = self.audit_log.write().await;
        log.push(entry);
        if log.len() > MAX_AUDIT_LOG_ENTRIES {
            let to_trim = log.len() - MAX_AUDIT_LOG_ENTRIES;
            log.drain(0..to_trim);
        }
    }

    async fn persist(&self) -> Result<(), CoreError> {
        let Ok(_guard) =
            tokio::time::timeout(std::time::Duration::from_secs(5), self.write_lock.lock()).await
        else {
            tracing::warn!("ban persistence lock timed out, skipping this persist cycle");
            return Ok(());
        };
        let audit_log = self.audit_log.read().await;
        let data = self.serialize_all(&audit_log)?;
        drop(audit_log);

        let tmp_path = self.file_path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, &data).await?;
        tokio::fs::rename(&tmp_path, &self.file_path).await?;

        Ok(())
    }
}

fn restore(table: &mut BanTable, stored: StoredBan, id: u64) {
    let entry = stored.into_entry(id);
    match key_of(&entry.target) {
        Some(key) => table.insert(key, id, entry),
        None => tracing::warn!(target = %entry.target, "unknown ban target type, skipping"),
    }
}

fn audit(action: BanAction, entry: &BanEntry, source: &BanSource) -> BanAuditLogEntry {
    BanAuditLogEntry {
        action,
        target: entry.target.clone(),
        reason: match action {
            BanAction::Ban => entry.reason.clone(),
            _ => None,
        },
        source: source.to_string(),
        timestamp: SystemTime::now(),
    }
}

impl BanStorage for FileBanStorage {
    fn load(&self) -> StorageFuture<'_, ()> {
        Box::pin(async move {
            match tokio::fs::read_to_string(&self.file_path).await {
                Ok(contents) => {
                    match serde_json::from_str::<BanFileData>(&contents) {
                        Ok(mut data) => {
                            let audit_log = std::mem::take(&mut data.audit_log);
                            self.populate(data);
                            *self.audit_log.write().await = audit_log;
                            tracing::info!(
                                path = %self.file_path.display(),
                                bans = self.read().by_id.len(),
                                "loaded ban data"
                            );
                        }
                        Err(e) => {
                            let backup = self.file_path.with_extension("json.bak");
                            tracing::warn!(
                                path = %self.file_path.display(),
                                error = %e,
                                backup = %backup.display(),
                                "ban file is corrupt, backing up and starting empty"
                            );
                            if let Err(rename_err) =
                                tokio::fs::rename(&self.file_path, &backup).await
                            {
                                tracing::warn!(error = %rename_err, "failed to back up corrupt ban file");
                            }
                        }
                    }
                    Ok(())
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tracing::debug!(path = %self.file_path.display(), "ban file not found, starting empty");
                    Ok(())
                }
                Err(e) => Err(CoreError::Io(e)),
            }
        })
    }

    fn save(&self) -> StorageFuture<'_, ()> {
        Box::pin(async move { self.persist().await })
    }

    fn add_ban(&self, entry: BanEntry) -> StorageFuture<'_, BanEntry> {
        Box::pin(async move {
            let Some(key) = key_of(&entry.target) else {
                return Err(CoreError::Other(format!(
                    "unsupported ban target type: {}",
                    entry.target
                )));
            };
            let mut entry = entry;
            entry.target = entry.target.canonical();
            {
                let mut table = self.write();
                let id = table.last_id + 1;
                entry.id = id.to_string();
                table.insert(key, id, entry.clone());
            }

            self.add_audit_entry(audit(BanAction::Ban, &entry, &entry.source))
                .await;
            self.persist().await?;

            tracing::info!(id = %entry.id, target = %entry.target, source = %entry.source, "ban added");
            Ok(entry)
        })
    }

    fn remove_ban<'a>(
        &'a self,
        target: &'a BanTarget,
        source: &'a BanSource,
    ) -> StorageFuture<'a, Option<BanEntry>> {
        Box::pin(async move {
            let Some(key) = key_of(target) else {
                return Ok(None);
            };
            let Some(entry) = self.write().remove(&key) else {
                return Ok(None);
            };

            self.add_audit_entry(audit(BanAction::Unban, &entry, source))
                .await;
            self.persist().await?;
            tracing::info!(id = %entry.id, target = %entry.target, source = %source, "ban removed");

            Ok((!entry.is_expired()).then_some(entry))
        })
    }

    fn get_ban<'a>(&'a self, target: &'a BanTarget) -> StorageFuture<'a, Option<BanEntry>> {
        Box::pin(
            async move { Ok(key_of(target).and_then(|key| self.read().active(&key).cloned())) },
        )
    }

    fn check<'a>(&'a self, attempt: &'a LoginAttempt) -> StorageFuture<'a, Option<BanEntry>> {
        Box::pin(async move { Ok(self.read().check(attempt)) })
    }

    fn list(&self, query: BanQuery) -> StorageFuture<'_, BanPage> {
        Box::pin(async move {
            let after =
                match query.cursor.as_deref() {
                    None => None,
                    Some(cursor) => Some(cursor.parse::<u64>().map_err(|_| {
                        CoreError::Other(format!("invalid ban list cursor: {cursor}"))
                    })?),
                };
            Ok(self.read().page(after, query.effective_limit()))
        })
    }

    fn get_all_active(&self) -> StorageFuture<'_, Vec<BanEntry>> {
        Box::pin(async move {
            Ok(self
                .read()
                .by_id
                .values()
                .filter(|entry| !entry.is_expired())
                .cloned()
                .collect())
        })
    }

    fn purge_expired(&self) -> StorageFuture<'_, usize> {
        Box::pin(async move {
            let purged = self.write().remove_expired();
            if purged.is_empty() {
                return Ok(0);
            }
            for entry in &purged {
                self.add_audit_entry(audit(BanAction::Expired, entry, &BanSource::System))
                    .await;
            }
            self.persist().await?;
            Ok(purged.len())
        })
    }
}
