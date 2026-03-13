use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use infrarust_config::{ConfigError, ServerConfig};

use crate::provider::ConfigChange;

/// Configuration provider that loads server configs from TOML files.
pub struct FileProvider {
    servers_dir: PathBuf,
}

impl FileProvider {
    /// Creates a new file provider for the given directory.
    pub fn new(servers_dir: PathBuf) -> Self {
        Self { servers_dir }
    }

    /// Loads all `.toml` server config files from the servers directory.
    ///
    /// Each file's stem (name without extension) becomes the config id.
    pub fn load_configs(&self) -> Result<Vec<ServerConfig>, ConfigError> {
        load_all_configs(&self.servers_dir)
    }

    /// Starts watching the servers directory for changes.
    ///
    /// Returns a receiver that emits `ConfigChange::FullReload` on any file change,
    /// and a handle to the watcher (must be kept alive).
    ///
    /// Phase 1 simplification: all changes trigger a full reload.
    pub fn watch(&self) -> Result<(mpsc::Receiver<ConfigChange>, RecommendedWatcher), ConfigError> {
        let (change_tx, change_rx) = mpsc::channel(16);
        let servers_dir = self.servers_dir.clone();

        // Notify signals via an unbounded channel (send from sync callback)
        let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<()>();

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
            .map_err(|e| ConfigError::Validation(format!("failed to create watcher: {e}")))?;

        watcher
            .watch(&self.servers_dir, RecursiveMode::NonRecursive)
            .map_err(|e| ConfigError::Validation(format!("failed to watch directory: {e}")))?;

        // Spawn a task to debounce events and reload configs
        tokio::spawn(async move {
            while notify_rx.recv().await.is_some() {
                // Debounce: wait briefly to batch rapid changes
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Drain any additional pending events
                while notify_rx.try_recv().is_ok() {}

                // Reload all configs
                match load_all_configs(&servers_dir) {
                    Ok(configs) => {
                        tracing::info!(count = configs.len(), "config change detected, reloading");
                        if change_tx
                            .send(ConfigChange::FullReload(configs))
                            .await
                            .is_err()
                        {
                            break; // Receiver dropped
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to reload configs");
                    }
                }
            }
        });

        Ok((change_rx, watcher))
    }
}

/// Loads all configs from a directory.
fn load_all_configs(dir: &Path) -> Result<Vec<ServerConfig>, ConfigError> {
    if !dir.exists() {
        return Err(ConfigError::DirectoryNotFound(dir.to_path_buf()));
    }

    let mut configs = Vec::new();

    let entries = std::fs::read_dir(dir).map_err(|source| ConfigError::ReadFile {
        path: dir.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| ConfigError::ReadFile {
            path: dir.to_path_buf(),
            source,
        })?;

        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            match load_server_config(&path) {
                Ok(config) => configs.push(config),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to load config");
                }
            }
        }
    }

    tracing::info!(
        dir = %dir.display(),
        count = configs.len(),
        "loaded server configs"
    );

    Ok(configs)
}

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
