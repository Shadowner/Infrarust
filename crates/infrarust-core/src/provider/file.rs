//! File-based configuration provider.
//!
//! Scans a directory of `.toml` files, loading each as a `ServerConfig`.
//! Watches for changes via `notify` with 200ms debouncing and emits
//! incremental `ProviderEvent`s (Added/Updated/Removed).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_config::{ConfigError, ServerConfig};

use crate::error::CoreError;
use crate::provider::{ConfigProvider, ProviderConfig, ProviderEvent, ProviderId};

/// Configuration provider that loads server configs from TOML files.
///
/// Each `.toml` file in the servers directory becomes a `ServerConfig`
/// identified as `file@<filename>`.
pub struct FileProvider {
    servers_dir: PathBuf,
}

impl FileProvider {
    /// Creates a new file provider for the given directory.
    pub const fn new(servers_dir: PathBuf) -> Self {
        Self { servers_dir }
    }
}

// ---------------------------------------------------------------------------
// ConfigProvider trait implementation
// ---------------------------------------------------------------------------

impl ConfigProvider for FileProvider {
    fn provider_type(&self) -> &str {
        "file"
    }

    fn load_initial(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<ProviderConfig>, CoreError>> + Send + '_>,
    > {
        Box::pin(self.do_load_initial())
    }

    fn watch(
        &self,
        sender: mpsc::Sender<ProviderEvent>,
        shutdown: CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + '_>>
    {
        Box::pin(self.do_watch(sender, shutdown))
    }
}

impl FileProvider {
    async fn do_load_initial(&self) -> Result<Vec<ProviderConfig>, CoreError> {
        let dir = &self.servers_dir;
        if !dir.exists() {
            tracing::warn!(dir = %dir.display(), "servers directory not found, returning empty");
            return Ok(Vec::new());
        }

        let mut configs = Vec::new();
        let entries = std::fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_none_or(|ext| ext != "toml") {
                continue;
            }

            match load_server_config(&path) {
                Ok(config) => {
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown.toml");
                    configs.push(ProviderConfig {
                        id: ProviderId::file(filename),
                        config,
                    });
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping invalid config");
                }
            }
        }

        tracing::info!(
            dir = %dir.display(),
            count = configs.len(),
            "file provider loaded initial configs"
        );
        Ok(configs)
    }

    async fn do_watch(
        &self,
        sender: mpsc::Sender<ProviderEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        // Build initial known map
        let mut known: HashMap<PathBuf, ServerConfig> = HashMap::new();
        if self.servers_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&self.servers_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "toml")
                    && let Ok(config) = load_server_config(&path)
                {
                    known.insert(path, config);
                }
            }
        }

        // Set up notify watcher
        let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<()>();
        let _watcher = {
            let mut watcher: RecommendedWatcher =
                notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                    if let Ok(event) = res {
                        use notify::EventKind;
                        match event.kind {
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                                let _ = notify_tx.send(());
                            }
                            _ => {}
                        }
                    }
                })
                .map_err(|e| CoreError::Other(format!("failed to create watcher: {e}")))?;

            watcher
                .watch(&self.servers_dir, RecursiveMode::NonRecursive)
                .map_err(|e| CoreError::Other(format!("failed to watch directory: {e}")))?;

            watcher // must be kept alive
        };

        loop {
            tokio::select! {
                recv = notify_rx.recv() => {
                    if recv.is_none() {
                        break; // Channel closed
                    }

                    // Debounce: wait 200ms and drain queued events
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    while notify_rx.try_recv().is_ok() {}

                    // Diff current directory state against known map
                    let events = compute_diff(&self.servers_dir, &mut known);
                    for event in events {
                        if sender.send(event).await.is_err() {
                            return Ok(()); // Receiver dropped
                        }
                    }
                }
                () = shutdown.cancelled() => {
                    tracing::debug!("file provider watch shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
}

/// Computes the diff between the current directory and the known map.
///
/// Returns a list of `ProviderEvent`s and updates the known map in-place.
fn compute_diff(dir: &Path, known: &mut HashMap<PathBuf, ServerConfig>) -> Vec<ProviderEvent> {
    let mut events = Vec::new();

    // Collect current files
    let mut current_files: HashMap<PathBuf, Option<ServerConfig>> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                let config = load_server_config(&path).ok();
                current_files.insert(path, config);
            }
        }
    }

    // Check for new and updated files
    for (path, maybe_config) in &current_files {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.toml");

        let Some(config) = maybe_config else {
            // Parse failed — log and keep old config if it exists
            tracing::warn!(path = %path.display(), "config parse failed, keeping previous version");
            continue;
        };

        if let Some(old_config) = known.get(path) {
            // File exists in both — check if changed
            if !configs_equal(old_config, config) {
                events.push(ProviderEvent::Updated(ProviderConfig {
                    id: ProviderId::file(filename),
                    config: config.clone(),
                }));
                known.insert(path.clone(), config.clone());
            }
        } else {
            // New file
            events.push(ProviderEvent::Added(ProviderConfig {
                id: ProviderId::file(filename),
                config: config.clone(),
            }));
            known.insert(path.clone(), config.clone());
        }
    }

    // Check for removed files
    let removed_paths: Vec<PathBuf> = known
        .keys()
        .filter(|path| !current_files.contains_key(*path))
        .cloned()
        .collect();

    for path in removed_paths {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.toml");
        events.push(ProviderEvent::Removed(ProviderId::file(filename)));
        known.remove(&path);
    }

    events
}

/// Compares two configs by their serializable fields to detect changes.
///
/// Uses a simple domain+address comparison since ServerConfig doesn't
/// derive PartialEq. A content hash would be more robust but this covers
/// the common cases.
fn configs_equal(a: &ServerConfig, b: &ServerConfig) -> bool {
    // Compare the most common change indicators
    a.domains == b.domains
        && format!("{:?}", a.addresses) == format!("{:?}", b.addresses)
        && a.proxy_mode == b.proxy_mode
        && a.send_proxy_protocol == b.send_proxy_protocol
        && a.max_players == b.max_players
        && a.id == b.id
        && format!("{:?}", a.motd) == format!("{:?}", b.motd)
        && format!("{:?}", a.server_manager) == format!("{:?}", b.server_manager)
        && format!("{:?}", a.ip_filter) == format!("{:?}", b.ip_filter)
        && a.disconnect_message == b.disconnect_message
        && format!("{:?}", a.domain_rewrite) == format!("{:?}", b.domain_rewrite)
        && format!("{:?}", a.timeouts) == format!("{:?}", b.timeouts)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Loads a single server config from a TOML file.
fn load_server_config(path: &Path) -> Result<ServerConfig, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;

    let mut config: ServerConfig =
        toml::from_str(&content).map_err(|source| ConfigError::ParseToml {
            path: path.to_path_buf(),
            source,
        })?;

    // Set id from filename if not explicitly set
    if config.id.is_none()
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        config.id = Some(stem.to_string());
    }

    Ok(config)
}
