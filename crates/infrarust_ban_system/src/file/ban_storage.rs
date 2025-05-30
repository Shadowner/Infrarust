use std::{num::NonZero, path::PathBuf, sync::Arc};

use lru::LruCache;
use tokio::{
    fs,
    sync::{Mutex, RwLock},
};

use crate::{
    BanAuditLogEntry, BanEntry, BanError,
    file::{AuditLogFileStorage, BanFileStorage},
    index::BanIndex,
};
use tracing::{debug, error, info};

/// File-based storage for ban data
#[derive(Debug)]
pub struct FileBanStorage {
    pub(super) bans_path: PathBuf,
    pub(super) audit_logs_path: Option<PathBuf>,

    pub(super) index: BanIndex,
    pub(super) bans_file_lock: Arc<Mutex<()>>,
    pub(super) audit_logs_file_lock: Arc<Mutex<()>>,

    #[allow(dead_code)]
    pub(super) cache: Arc<RwLock<LruCache<String, Arc<BanEntry>>>>,
    pub(super) bans_dirty: Arc<RwLock<bool>>,
    pub(super) combined_storage: bool,
}

impl FileBanStorage {
    pub async fn new(
        bans_path: &str,
        audit_logs_path_opt: Option<&str>,
        cache_size: usize,
    ) -> Result<Self, BanError> {
        let bans_path = PathBuf::from(bans_path);

        // Determine the audit logs path:
        // - If explicitly provided, use that
        // - If empty or None, use bans_path with ".audit.json" extension
        let (audit_logs_path, combined_storage) = match audit_logs_path_opt {
            Some(path) if !path.is_empty() => (PathBuf::from(path), false),
            _ => {
                let mut path = bans_path.clone();
                let stem = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                path.set_file_name(format!("{}.audit.json", stem));

                (path, false)
            }
        };

        // Create parent directories if they don't exist
        for path in [&bans_path, &audit_logs_path] {
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    Self::create_directory(parent).await?;
                }
            }
        }

        let storage = Self {
            bans_path: bans_path.clone(),
            audit_logs_path: Some(audit_logs_path.clone()),
            index: BanIndex::new(),
            bans_file_lock: Arc::new(Mutex::new(())),
            audit_logs_file_lock: Arc::new(Mutex::new(())),
            cache: Arc::new(RwLock::new(LruCache::new(
                NonZero::new(cache_size).unwrap(),
            ))),
            bans_dirty: Arc::new(RwLock::new(false)),
            combined_storage,
        };

        if bans_path.exists() {
            storage.load_bans_from_file().await?;
        } else {
            storage.save_bans_to_file().await?;
        }

        // Initialize audit logs file if it doesn't exist
        if !audit_logs_path.exists() && !combined_storage {
            storage.save_empty_audit_logs_file().await?;
        }

        storage.start_background_savers();
        Ok(storage)
    }

    async fn create_directory(path: &std::path::Path) -> Result<(), BanError> {
        fs::create_dir_all(path).await.map_err(|e| {
            error!("Failed to create directories {}: {}", path.display(), e);
            BanError::Io(e)
        })
    }

    async fn read_json_file<T: serde::de::DeserializeOwned>(
        &self,
        path: &PathBuf,
        error_context: &str,
    ) -> Result<T, BanError> {
        let content = fs::read_to_string(path).await.map_err(|e| {
            error!("Failed to read {}: {}", error_context, e);
            BanError::Io(e)
        })?;

        serde_json::from_str(&content).map_err(|e| {
            error!("Failed to parse {}: {}", error_context, e);
            BanError::Serialization(e.to_string())
        })
    }

    async fn write_json_file<T: serde::Serialize>(
        &self,
        path: &PathBuf,
        data: &T,
        error_context: &str,
    ) -> Result<(), BanError> {
        let content = serde_json::to_string_pretty(&data).map_err(|e| {
            error!("Failed to serialize {}: {}", error_context, e);
            BanError::Serialization(e.to_string())
        })?;

        let temp_path = path.with_extension("tmp");

        if let Some(parent) = temp_path.parent() {
            if !parent.exists() {
                Self::create_directory(parent).await?;
            }
        }

        fs::write(&temp_path, content).await.map_err(|e| {
            error!(
                "Failed to write temporary file for {}: {}",
                error_context, e
            );
            BanError::Io(e)
        })?;

        fs::rename(&temp_path, path).await.map_err(|e| {
            error!(
                "Failed to rename temporary file for {}: {}",
                error_context, e
            );
            BanError::Io(e)
        })?;

        Ok(())
    }

    pub(super) async fn load_bans_from_file(&self) -> Result<(), BanError> {
        debug!("Loading ban data from file: {}", self.bans_path.display());

        let _guard = self.bans_file_lock.lock().await;
        let data: BanFileStorage = self
            .read_json_file(
                &self.bans_path,
                &format!("ban file {}", self.bans_path.display()),
            )
            .await?;

        for ban in data.bans {
            self.index.add(Arc::new(ban)).await;
        }

        info!("Loaded {} bans from file", self.index.count());
        Ok(())
    }

    pub(super) async fn save_empty_audit_logs_file(&self) -> Result<(), BanError> {
        if self.combined_storage || self.audit_logs_path.is_none() {
            return Ok(());
        }

        let path = self.audit_logs_path.as_ref().unwrap();
        debug!("Creating empty audit logs file: {}", path.display());

        let _guard = self.audit_logs_file_lock.lock().await;
        let data = AuditLogFileStorage {
            audit_logs: Vec::new(),
            format_version: 1,
        };

        self.write_json_file(path, &data, &format!("audit logs file {}", path.display()))
            .await?;

        debug!(
            log_type = "ban_system",
            "Successfully created empty audit logs file"
        );
        Ok(())
    }

    pub(super) async fn load_audit_logs_paged(
        &self,
        page: usize,
        page_size: usize,
    ) -> Result<(Vec<BanAuditLogEntry>, usize), BanError> {
        if self.combined_storage || self.audit_logs_path.is_none() {
            return Ok((Vec::new(), 0));
        }

        let path = self.audit_logs_path.as_ref().unwrap();
        debug!(
            "Loading audit logs page {} (size {}) from file: {}",
            page,
            page_size,
            path.display()
        );

        let _guard = self.audit_logs_file_lock.lock().await;

        if !path.exists() {
            debug!("Audit logs file does not exist, returning empty result");
            return Ok((Vec::new(), 0));
        }

        let data: AuditLogFileStorage = self
            .read_json_file(path, &format!("audit logs file {}", path.display()))
            .await?;

        let total = data.audit_logs.len();
        let start = page * page_size;
        let end = (start + page_size).min(total);

        if start >= total {
            return Ok((Vec::new(), total));
        }

        Ok((data.audit_logs[start..end].to_vec(), total))
    }

    pub(super) async fn append_audit_log(&self, entry: BanAuditLogEntry) -> Result<(), BanError> {
        self.append_audit_logs_batch(vec![entry]).await
    }

    pub(super) async fn append_audit_logs_batch(
        &self,
        entries: Vec<BanAuditLogEntry>,
    ) -> Result<(), BanError> {
        if entries.is_empty() {
            return Ok(());
        }

        if self.combined_storage {
            return self.append_audit_logs_batch_combined(entries).await;
        }

        if self.audit_logs_path.is_none() {
            return Ok(());
        }

        let path = self.audit_logs_path.as_ref().unwrap();
        debug!(
            "Appending batch of {} audit log entries to file: {}",
            entries.len(),
            path.display()
        );

        let _guard = self.audit_logs_file_lock.lock().await;

        // Create new file with entries if it doesn't exist
        if !path.exists() {
            let data = AuditLogFileStorage {
                audit_logs: entries,
                format_version: 1,
            };

            return self
                .write_json_file(path, &data, &format!("audit logs file {}", path.display()))
                .await;
        }

        // Update existing file
        let mut data: AuditLogFileStorage = self
            .read_json_file(path, &format!("audit logs file {}", path.display()))
            .await?;

        data.audit_logs.extend(entries);

        self.write_json_file(path, &data, &format!("audit logs file {}", path.display()))
            .await?;

        debug!(
            log_type = "ban_system",
            "Successfully appended batch of audit log entries"
        );
        Ok(())
    }

    pub(super) async fn append_audit_logs_batch_combined(
        &self,
        entries: Vec<BanAuditLogEntry>,
    ) -> Result<(), BanError> {
        if entries.is_empty() {
            return Ok(());
        }

        debug!(
            "Appending batch of {} audit log entries to combined storage file: {}",
            entries.len(),
            self.bans_path.display()
        );

        let _guard = self.bans_file_lock.lock().await;

        let mut data: BanFileStorage = self
            .read_json_file(
                &self.bans_path,
                &format!("ban file {}", self.bans_path.display()),
            )
            .await?;

        if data.audit_logs.is_none() {
            data.audit_logs = Some(entries);
        } else if let Some(logs) = &mut data.audit_logs {
            logs.extend(entries);
        }

        self.write_json_file(
            &self.bans_path,
            &data,
            &format!("ban file {}", self.bans_path.display()),
        )
        .await?;

        debug!(
            log_type = "ban_system",
            "Successfully appended batch of audit log entries to combined storage"
        );
        Ok(())
    }

    pub(super) async fn save_bans_to_file(&self) -> Result<(), BanError> {
        debug!("Saving ban data to file: {}", self.bans_path.display());

        let _guard = self.bans_file_lock.lock().await;
        self.save_bans_to_file_internal().await?;

        let mut dirty = self.bans_dirty.write().await;
        *dirty = false;

        debug!("Successfully saved ban data to file");
        Ok(())
    }

    async fn save_bans_to_file_internal(&self) -> Result<(), BanError> {
        let bans = self.index.get_all();
        let bans_vec: Vec<BanEntry> = bans.iter().map(|b| (**b).clone()).collect();

        let data = BanFileStorage {
            bans: bans_vec,
            audit_logs: None,
            format_version: 1,
        };

        self.write_json_file(
            &self.bans_path,
            &data,
            &format!("ban file {}", self.bans_path.display()),
        )
        .await
    }

    pub(super) fn start_background_savers(&self) {
        self.start_background_bans_saver();
    }

    pub(super) fn start_background_bans_saver(&self) {
        let path = self.bans_path.clone();
        let dirty = self.bans_dirty.clone();
        let index = self.index.clone();
        let file_lock = self.bans_file_lock.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));

            loop {
                interval.tick().await;

                let needs_saving = {
                    let dirty_val = dirty.read().await;
                    *dirty_val
                };

                if needs_saving {
                    debug!("Background save for ban data: {}", path.display());

                    let _guard = file_lock.lock().await;
                    let bans = index.get_all();
                    let bans_vec: Vec<BanEntry> = bans.iter().map(|b| (**b).clone()).collect();

                    let data = BanFileStorage {
                        bans: bans_vec,
                        audit_logs: None,
                        format_version: 1,
                    };

                    // Reusing the file writing logic from a helper method
                    let result = async {
                        let content = serde_json::to_string_pretty(&data).map_err(|e| {
                            error!("Failed to serialize ban data: {}", e);
                            BanError::Serialization(e.to_string())
                        })?;

                        let temp_path = path.with_extension("tmp");
                        if let Some(parent) = temp_path.parent() {
                            if !parent.exists() {
                                fs::create_dir_all(parent).await.map_err(|e| {
                                    error!(
                                        "Failed to create directories {}: {}",
                                        parent.display(),
                                        e
                                    );
                                    BanError::Io(e)
                                })?;
                            }
                        }

                        fs::write(&temp_path, content).await.map_err(|e| {
                            error!(
                                "Failed to write temporary ban file {}: {}",
                                temp_path.display(),
                                e
                            );
                            BanError::Io(e)
                        })?;

                        fs::rename(&temp_path, &path).await.map_err(|e| {
                            error!(
                                "Failed to rename temporary ban file to {}: {}",
                                path.display(),
                                e
                            );
                            BanError::Io(e)
                        })?;

                        Ok::<(), BanError>(())
                    }
                    .await;

                    if result.is_ok() {
                        let mut dirty_val = dirty.write().await;
                        *dirty_val = false;
                        debug!("Background save completed for ban data");
                    }
                }
            }
        });
    }

    pub(super) async fn mark_bans_dirty(&self) {
        let mut dirty = self.bans_dirty.write().await;
        *dirty = true;
    }
}
