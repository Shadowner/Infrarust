//! File-based ban storage with `DashMap` indexes and crash-safe JSON persistence.

use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::SystemTime;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ban::storage::BanStorage;
use crate::ban::types::{BanAction, BanAuditLogEntry, BanEntry, BanTarget};
use crate::error::CoreError;

/// JSON file structure for persistence.
#[derive(Serialize, Deserialize, Default)]
struct BanFileData {
    bans: Vec<BanEntry>,
    audit_log: Vec<BanAuditLogEntry>,
}

/// Ban storage backed by a JSON file with in-memory `DashMap` indexes.
///
/// Three `DashMaps` for O(1) lookup by target type.
/// Crash-safe persistence via temp file + atomic rename.
pub struct FileBanStorage {
    /// Index by IP address.
    ip_bans: DashMap<IpAddr, BanEntry>,
    /// Index by username (stored lowercase for case-insensitive search).
    username_bans: DashMap<String, BanEntry>,
    /// Index by UUID.
    uuid_bans: DashMap<Uuid, BanEntry>,
    /// Path to the persistence file.
    file_path: PathBuf,
    /// Audit log (append-only in memory, persisted with bans).
    audit_log: tokio::sync::RwLock<Vec<BanAuditLogEntry>>,
    /// Serializes file writes to prevent concurrent temp file conflicts.
    write_lock: tokio::sync::Mutex<()>,
}

const MAX_AUDIT_LOG_ENTRIES: usize = 10_000;

impl FileBanStorage {
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            ip_bans: DashMap::new(),
            username_bans: DashMap::new(),
            uuid_bans: DashMap::new(),
            file_path,
            audit_log: tokio::sync::RwLock::new(Vec::new()),
            write_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Serializes all data to JSON.
    fn serialize_all(&self, audit_log: &[BanAuditLogEntry]) -> Result<String, CoreError> {
        let mut bans = Vec::new();
        for entry in &self.ip_bans {
            bans.push(entry.value().clone());
        }
        for entry in &self.username_bans {
            bans.push(entry.value().clone());
        }
        for entry in &self.uuid_bans {
            bans.push(entry.value().clone());
        }

        let data = BanFileData {
            bans,
            audit_log: audit_log.to_vec(),
        };

        serde_json::to_string_pretty(&data).map_err(|e| CoreError::Other(e.to_string()))
    }

    /// Populates the `DashMaps` from a list of ban entries.
    fn populate_maps(&self, bans: Vec<BanEntry>) {
        for entry in bans {
            match &entry.target {
                BanTarget::Ip(ip) => {
                    self.ip_bans.insert(*ip, entry);
                }
                BanTarget::Username(name) => {
                    self.username_bans.insert(name.to_lowercase(), entry);
                }
                BanTarget::Uuid(uuid) => {
                    self.uuid_bans.insert(*uuid, entry);
                }
                _ => {
                    tracing::warn!(target = %entry.target, "unknown ban target type, skipping");
                }
            }
        }
    }

    /// Adds an audit log entry.
    async fn add_audit_entry(&self, entry: BanAuditLogEntry) {
        let mut log = self.audit_log.write().await;
        log.push(entry);
        if log.len() > MAX_AUDIT_LOG_ENTRIES {
            let to_trim = log.len() - MAX_AUDIT_LOG_ENTRIES;
            log.drain(0..to_trim);
        }
    }

    /// Persists to disk (crash-safe: write tmp then rename).
    /// Serialized with a mutex to prevent concurrent temp file conflicts.
    /// Uses a timeout to avoid blocking indefinitely on slow I/O.
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

impl BanStorage for FileBanStorage {
    fn load(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async move {
            match tokio::fs::read_to_string(&self.file_path).await {
                Ok(contents) => {
                    match serde_json::from_str::<BanFileData>(&contents) {
                        Ok(data) => {
                            self.populate_maps(data.bans);
                            {
                                let mut log = self.audit_log.write().await;
                                *log = data.audit_log;
                            }
                            tracing::info!(
                                path = %self.file_path.display(),
                                ip_bans = self.ip_bans.len(),
                                username_bans = self.username_bans.len(),
                                uuid_bans = self.uuid_bans.len(),
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

    fn save(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async move { self.persist().await })
    }

    fn add_ban(
        &self,
        entry: BanEntry,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async move {
            match &entry.target {
                BanTarget::Ip(ip) => {
                    self.ip_bans.insert(*ip, entry.clone());
                }
                BanTarget::Username(name) => {
                    self.username_bans
                        .insert(name.to_lowercase(), entry.clone());
                }
                BanTarget::Uuid(uuid) => {
                    self.uuid_bans.insert(*uuid, entry.clone());
                }
                _ => {
                    return Err(CoreError::Other(format!(
                        "unsupported ban target type: {}",
                        entry.target
                    )));
                }
            }

            self.add_audit_entry(BanAuditLogEntry {
                action: BanAction::Ban,
                target: entry.target.clone(),
                reason: entry.reason.clone(),
                source: entry.source.clone(),
                timestamp: SystemTime::now(),
            })
            .await;

            self.persist().await?;

            tracing::info!(target = %entry.target, source = %entry.source, "ban added");
            Ok(())
        })
    }

    fn remove_ban(
        &self,
        target: &BanTarget,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<bool, CoreError>> + Send + '_>> {
        let target = target.clone();
        Box::pin(async move {
            let removed = match &target {
                BanTarget::Ip(ip) => self.ip_bans.remove(ip).is_some(),
                BanTarget::Username(name) => {
                    self.username_bans.remove(&name.to_lowercase()).is_some()
                }
                BanTarget::Uuid(uuid) => self.uuid_bans.remove(uuid).is_some(),
                _ => false,
            };

            if removed {
                self.add_audit_entry(BanAuditLogEntry {
                    action: BanAction::Unban,
                    target: target.clone(),
                    reason: None,
                    source: String::new(),
                    timestamp: SystemTime::now(),
                })
                .await;

                self.persist().await?;
                tracing::info!(target = %target, "ban removed");
            }

            Ok(removed)
        })
    }

    fn is_banned(
        &self,
        target: &BanTarget,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Option<BanEntry>, CoreError>> + Send + '_>>
    {
        let target = target.clone();
        Box::pin(async move {
            match &target {
                BanTarget::Ip(ip) => {
                    if let Some(entry) = self.ip_bans.get(ip) {
                        if entry.is_expired() {
                            drop(entry);
                            self.ip_bans.remove(ip);
                            self.persist().await?;
                            return Ok(None);
                        }
                        return Ok(Some(entry.clone()));
                    }
                }
                BanTarget::Username(name) => {
                    let key = name.to_lowercase();
                    if let Some(entry) = self.username_bans.get(&key) {
                        if entry.is_expired() {
                            drop(entry);
                            self.username_bans.remove(&key);
                            self.persist().await?;
                            return Ok(None);
                        }
                        return Ok(Some(entry.clone()));
                    }
                }
                BanTarget::Uuid(uuid) => {
                    if let Some(entry) = self.uuid_bans.get(uuid) {
                        if entry.is_expired() {
                            drop(entry);
                            self.uuid_bans.remove(uuid);
                            self.persist().await?;
                            return Ok(None);
                        }
                        return Ok(Some(entry.clone()));
                    }
                }
                _ => {}
            }
            Ok(None)
        })
    }

    fn check_player<'a>(
        &'a self,
        ip: &'a IpAddr,
        username: &'a str,
        uuid: Option<&'a Uuid>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Option<BanEntry>, CoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            // 1. Check by IP
            if let Some(entry) = self.ip_bans.get(ip) {
                if !entry.is_expired() {
                    return Ok(Some(entry.clone()));
                }
                drop(entry);
                self.ip_bans.remove(ip);
            }

            // 2. Check by username (case-insensitive)
            let username_lower = username.to_lowercase();
            if let Some(entry) = self.username_bans.get(&username_lower) {
                if !entry.is_expired() {
                    return Ok(Some(entry.clone()));
                }
                drop(entry);
                self.username_bans.remove(&username_lower);
            }

            // 3. Check by UUID (if available)
            if let Some(uuid) = uuid
                && let Some(entry) = self.uuid_bans.get(uuid)
            {
                if !entry.is_expired() {
                    return Ok(Some(entry.clone()));
                }
                drop(entry);
                self.uuid_bans.remove(uuid);
            }

            Ok(None)
        })
    }

    fn get_all_active(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<BanEntry>, CoreError>> + Send + '_>>
    {
        Box::pin(async move {
            let mut active = Vec::new();
            for entry in &self.ip_bans {
                if !entry.is_expired() {
                    active.push(entry.clone());
                }
            }
            for entry in &self.username_bans {
                if !entry.is_expired() {
                    active.push(entry.clone());
                }
            }
            for entry in &self.uuid_bans {
                if !entry.is_expired() {
                    active.push(entry.clone());
                }
            }
            Ok(active)
        })
    }

    fn purge_expired(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<usize, CoreError>> + Send + '_>> {
        Box::pin(async move {
            let mut purged = 0usize;
            let mut purged_targets = Vec::new();

            // Collect expired keys first, then remove (avoid DashMap deadlock)
            let expired_ips: Vec<IpAddr> = self
                .ip_bans
                .iter()
                .filter(|e| e.is_expired())
                .map(|e| *e.key())
                .collect();
            for ip in &expired_ips {
                self.ip_bans.remove(ip);
                purged_targets.push(BanTarget::Ip(*ip));
            }
            purged += expired_ips.len();

            let expired_usernames: Vec<String> = self
                .username_bans
                .iter()
                .filter(|e| e.is_expired())
                .map(|e| e.key().clone())
                .collect();
            for name in &expired_usernames {
                self.username_bans.remove(name);
                purged_targets.push(BanTarget::Username(name.clone()));
            }
            purged += expired_usernames.len();

            let expired_uuids: Vec<Uuid> = self
                .uuid_bans
                .iter()
                .filter(|e| e.is_expired())
                .map(|e| *e.key())
                .collect();
            for uuid in &expired_uuids {
                self.uuid_bans.remove(uuid);
                purged_targets.push(BanTarget::Uuid(*uuid));
            }
            purged += expired_uuids.len();

            if purged > 0 {
                // Add audit entries for expired bans
                {
                    let mut log = self.audit_log.write().await;
                    for target in purged_targets {
                        log.push(BanAuditLogEntry {
                            action: BanAction::Expired,
                            target,
                            reason: None,
                            source: "system".to_string(),
                            timestamp: SystemTime::now(),
                        });
                    }
                }
                self.persist().await?;
            }

            Ok(purged)
        })
    }
}
