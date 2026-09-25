//! File-based configuration provider.
//!
//! Scans a directory of `.toml` files, loading each as a `ServerConfig`.
//! Watches for changes via `notify` and emits incremental `ProviderEvent`s
//! (Added/Updated/Removed).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use infrarust_config::{ConfigError, ServerConfig};

use crate::error::CoreError;
use crate::provider::{ConfigProvider, ProviderConfig, ProviderEvent, ProviderId};

const QUIET_PERIOD: Duration = Duration::from_millis(300);
const MAX_WAIT: Duration = Duration::from_secs(2);

/// Configuration provider that loads server configs from TOML files.
///
/// Each `.toml` file in the servers directory becomes a `ServerConfig`
/// identified as `file@<filename>`.
pub struct FileProvider {
    servers_dir: PathBuf,
    published: Mutex<Option<HashMap<PathBuf, ServerConfig>>>,
}

impl FileProvider {
    pub const fn new(servers_dir: PathBuf) -> Self {
        Self {
            servers_dir,
            published: Mutex::new(None),
        }
    }

    fn lock_published(&self) -> MutexGuard<'_, Option<HashMap<PathBuf, ServerConfig>>> {
        self.published
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl ConfigProvider for FileProvider {
    fn provider_type(&self) -> &'static str {
        "file"
    }

    fn load_initial(
        &self,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Vec<ProviderConfig>, CoreError>> + Send + '_>>
    {
        Box::pin(async move { self.do_load_initial() })
    }

    fn watch(
        &self,
        sender: mpsc::Sender<ProviderEvent>,
        shutdown: CancellationToken,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(self.do_watch(sender, shutdown))
    }
}

impl FileProvider {
    fn do_load_initial(&self) -> Result<Vec<ProviderConfig>, CoreError> {
        let dir = &self.servers_dir;
        if !dir.exists() {
            tracing::warn!(dir = %dir.display(), "servers directory not found, returning empty");
            *self.lock_published() = Some(HashMap::new());
            return Ok(Vec::new());
        }

        let mut loaded = HashMap::new();
        let entries = std::fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_none_or(|ext| ext != "toml") {
                continue;
            }

            match load_server_config(&path) {
                Ok(config) => {
                    loaded.insert(path, config);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping invalid config");
                }
            }
        }

        let configs: Vec<ProviderConfig> = loaded
            .iter()
            .map(|(path, config)| ProviderConfig {
                id: file_id(path),
                config: config.clone(),
            })
            .collect();
        *self.lock_published() = Some(loaded);

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
        let (changes_tx, changes) = mpsc::unbounded_channel::<()>();
        let _watcher = watch_directory(&self.servers_dir, changes_tx)?;
        let published = self.lock_published().take().unwrap_or_default();
        follow(&self.servers_dir, published, changes, &sender, &shutdown).await;
        Ok(())
    }
}

fn watch_directory(
    dir: &Path,
    changes: mpsc::UnboundedSender<()>,
) -> Result<RecommendedWatcher, CoreError> {
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                use notify::EventKind;
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                        let _ = changes.send(());
                    }
                    _ => {}
                }
            }
        })
        .map_err(|e| CoreError::Other(format!("failed to create watcher: {e}")))?;

    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .map_err(|e| CoreError::Other(format!("failed to watch directory: {e}")))?;

    Ok(watcher)
}

async fn follow(
    dir: &Path,
    mut known: HashMap<PathBuf, ServerConfig>,
    mut changes: mpsc::UnboundedReceiver<()>,
    sender: &mpsc::Sender<ProviderEvent>,
    shutdown: &CancellationToken,
) {
    if !publish(compute_diff(dir, &mut known), sender).await {
        return;
    }

    let mut pending: Option<Pending> = None;
    loop {
        let reload_at = pending.as_ref().map(Pending::reload_at);
        tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                tracing::debug!("file provider watch shutting down");
                return;
            }
            () = sleep_until_some(reload_at) => {
                pending = None;
                if !publish(compute_diff(dir, &mut known), sender).await {
                    return;
                }
            }
            change = changes.recv() => {
                if change.is_none() {
                    return;
                }
                let now = Instant::now();
                pending = Some(Pending {
                    quiet_until: now + QUIET_PERIOD,
                    deadline: pending.map_or(now + MAX_WAIT, |p| p.deadline),
                });
            }
        }
    }
}

struct Pending {
    quiet_until: Instant,
    deadline: Instant,
}

impl Pending {
    fn reload_at(&self) -> Instant {
        self.quiet_until.min(self.deadline)
    }
}

async fn sleep_until_some(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn publish(events: Vec<ProviderEvent>, sender: &mpsc::Sender<ProviderEvent>) -> bool {
    for event in events {
        if sender.send(event).await.is_err() {
            return false;
        }
    }
    true
}

fn file_id(path: &Path) -> ProviderId {
    ProviderId::file(
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.toml"),
    )
}

/// Computes the diff between the current directory and the known map.
///
/// Returns a list of `ProviderEvent`s and updates the known map in-place.
fn compute_diff(dir: &Path, known: &mut HashMap<PathBuf, ServerConfig>) -> Vec<ProviderEvent> {
    let mut events = Vec::new();

    // Collect current files
    let entries = match std::fs::read_dir(dir).and_then(Iterator::collect::<Result<Vec<_>, _>>) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                dir = %dir.display(),
                error = %e,
                "cannot list the servers directory, keeping the current servers"
            );
            return events;
        }
    };
    let mut current_files: HashMap<PathBuf, Option<ServerConfig>> = HashMap::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            let config = load_server_config(&path).ok();
            current_files.insert(path, config);
        }
    }

    // Check for new and updated files
    for (path, maybe_config) in &current_files {
        let Some(config) = maybe_config else {
            // Parse failed — log and keep old config if it exists
            tracing::warn!(path = %path.display(), "config parse failed, keeping previous version");
            continue;
        };

        if let Some(old_config) = known.get(path) {
            // File exists in both — check if changed
            if old_config != config {
                events.push(ProviderEvent::Updated(ProviderConfig {
                    id: file_id(path),
                    config: config.clone(),
                }));
                known.insert(path.clone(), config.clone());
            }
        } else {
            // New file
            events.push(ProviderEvent::Added(ProviderConfig {
                id: file_id(path),
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
        events.push(ProviderEvent::Removed(file_id(&path)));
        known.remove(&path);
    }

    events
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

    infrarust_config::validate_server_config(&config)?;

    // Set id from filename if not explicitly set
    if config.id.is_none()
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        config.id = Some(stem.to_string());
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use tokio::task::JoinHandle;

    const SERVER: &str = "server.toml";

    fn offline(address: &str, domains: &str) -> String {
        format!("proxy_mode = \"offline\"\naddresses = [\"{address}\"]\n{domains}")
    }

    struct Follower {
        dir: tempfile::TempDir,
        start: Instant,
        changes: mpsc::UnboundedSender<()>,
        shutdown: CancellationToken,
        received: JoinHandle<Vec<(u64, String)>>,
    }

    impl Follower {
        async fn start(initial: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(SERVER);
            std::fs::write(&path, initial).unwrap();
            let known = HashMap::from([(path.clone(), load_server_config(&path).unwrap())]);

            let start = Instant::now() + Duration::from_secs(1);
            let (changes, changes_rx) = mpsc::unbounded_channel();
            let (tx, mut rx) = mpsc::channel(32);
            let shutdown = CancellationToken::new();

            let watched = dir.path().to_path_buf();
            let token = shutdown.clone();
            tokio::spawn(async move {
                follow(&watched, known, changes_rx, &tx, &token).await;
            });
            let received = tokio::spawn(async move {
                let mut received = Vec::new();
                while let Some(event) = rx.recv().await {
                    let at = u64::try_from(start.elapsed().as_millis()).unwrap();
                    received.push((at, describe(&event)));
                }
                received
            });
            tokio::time::sleep_until(start).await;

            Self {
                dir,
                start,
                changes,
                shutdown,
                received,
            }
        }

        async fn at(&self, millis: u64) {
            tokio::time::sleep_until(self.start + Duration::from_millis(millis)).await;
        }

        fn write(&self, content: &str) {
            std::fs::write(self.dir.path().join(SERVER), content).unwrap();
            self.changes.send(()).unwrap();
        }

        fn append(&self, content: &str) {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(self.dir.path().join(SERVER))
                .unwrap();
            file.write_all(content.as_bytes()).unwrap();
            self.changes.send(()).unwrap();
        }

        async fn finish(self, millis: u64) -> Vec<(u64, String)> {
            self.at(millis).await;
            self.shutdown.cancel();
            self.received.await.unwrap()
        }
    }

    fn describe(event: &ProviderEvent) -> String {
        match event {
            ProviderEvent::Added(pc) | ProviderEvent::Updated(pc) => format!(
                "{} {:?} {}",
                pc.id, pc.config.domains, pc.config.addresses[0].address.port
            ),
            ProviderEvent::Removed(id) => format!("{id} removed"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_changes_is_read_once_it_goes_quiet() {
        let follower =
            Follower::start(&offline("127.0.0.1:25565", "domains = [\"lobby.test\"]\n")).await;

        follower.write(&offline("127.0.0.1:1000", "domains = [\"lobby.test\"]\n"));
        follower.at(150).await;
        follower.write(&offline("127.0.0.1:1150", "domains = [\"lobby.test\"]\n"));
        follower.at(300).await;
        follower.write(&offline("127.0.0.1:1300", "domains = [\"lobby.test\"]\n"));

        assert_eq!(
            follower.finish(3_000).await,
            [(600, "file@server.toml [\"lobby.test\"] 1300".to_string())]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_file_written_in_two_halves_is_read_once_complete() {
        let follower =
            Follower::start(&offline("127.0.0.1:25565", "domains = [\"lobby.test\"]\n")).await;

        follower.write(&offline("127.0.0.1:25566", ""));
        follower.at(250).await;
        follower.append("domains = [\"lobby.test\"]\n");

        assert_eq!(
            follower.finish(3_000).await,
            [(550, "file@server.toml [\"lobby.test\"] 25566".to_string())]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_file_that_never_goes_quiet_is_still_read() {
        let follower =
            Follower::start(&offline("127.0.0.1:25565", "domains = [\"lobby.test\"]\n")).await;

        for step in 0..=18_u16 {
            follower.at(u64::from(step) * 150).await;
            let port = 1000 + step;
            follower.write(&offline(
                &format!("127.0.0.1:{port}"),
                "domains = [\"lobby.test\"]\n",
            ));
        }

        assert_eq!(
            follower.finish(5_000).await,
            [
                (2_000, "file@server.toml [\"lobby.test\"] 1013".to_string()),
                (3_000, "file@server.toml [\"lobby.test\"] 1018".to_string()),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unparsable_file_keeps_the_previous_version() {
        let follower =
            Follower::start(&offline("127.0.0.1:25565", "domains = [\"lobby.test\"]\n")).await;

        follower.write("proxy_mode = \"offline\"\naddresses = [\"127.0.0");
        follower.at(1_000).await;
        follower.write(&offline("127.0.0.1:25566", "domains = [\"lobby.test\"]\n"));

        assert_eq!(
            follower.finish(3_000).await,
            [(1_300, "file@server.toml [\"lobby.test\"] 25566".to_string())]
        );
    }
}
