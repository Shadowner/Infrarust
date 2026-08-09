//! `<data_dir>/servers/` — the TOML files this plugin owns.
//!
//! Every server created through the REST API lives here as a full
//! `infrarust_config::ServerConfig` document. The proxy's own `servers_dir` is
//! never touched. Changes reach the proxy as [`ServerDocument`] events, either
//! from the filesystem watcher or from an explicit reload.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::provider::{PluginProviderEvent, PluginProviderSender, ServerDocument};
use infrarust_api::types::ServerId;
use infrarust_config::ServerConfig;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const SERVERS_SUBDIR: &str = "servers";
const LEGACY_STORE: &str = "servers.json";
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Slot holding the sender the proxy hands to [`crate::api_provider::ApiConfigProvider::watch`].
pub type ProviderSenderSlot = tokio::sync::Mutex<Option<Box<dyn PluginProviderSender>>>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("server id '{0}' cannot be mapped to a file name")]
    InvalidId(String),

    #[error("a document is already filed as '{0}'")]
    Occupied(String),

    #[error("invalid server document: {0}")]
    Document(String),

    #[error("a document filed as '{id}' cannot identify as '{claimed}'")]
    IdMismatch { id: String, claimed: String },

    #[error("failed to serialize server '{id}': {source}")]
    Serialize {
        id: String,
        source: toml::ser::Error,
    },

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Longest id that fits a document name, matching
/// `infrarust_config::validate_server_config`.
pub const MAX_ID_LEN: usize = 64;

/// The name a document is filed under. It is at once the file stem, the id
/// the proxy knows the document by, and — for every document this plugin
/// writes — the config's own `effective_id()`: [`ServerDir`] derives it from
/// the config being stored, so the two can never drift apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DocumentId(String);

impl DocumentId {
    /// Accepts the ids `infrarust_config` accepts, minus the dot placements
    /// that would not survive a round trip through a file name.
    pub fn new(id: &str) -> Option<Self> {
        let usable = !id.is_empty()
            && id.len() <= MAX_ID_LEN
            && !id.starts_with('.')
            && !id.contains("..")
            && id.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
            });

        usable.then(|| Self(id.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a server id relates to the documents in the directory.
#[derive(Debug, PartialEq, Eq)]
pub enum Ownership {
    /// `<id>.toml` exists and its config identifies as `id`.
    Owned,
    /// Another stem claims `id`, or `<id>.toml` claims another id. Writing
    /// `<id>.toml` would leave two documents fighting over one server.
    Shadowed,
    /// No document in the directory claims `id`.
    Absent,
}

/// A completed write, holding the directory's write lock until the caller has
/// told the proxy about it, so concurrent writers reach the router in the same
/// order they reached the disk.
#[derive(Debug)]
pub struct Committed<'a> {
    id: DocumentId,
    _guard: tokio::sync::MutexGuard<'a, ()>,
}

impl Committed<'_> {
    pub fn id(&self) -> &DocumentId {
        &self.id
    }
}

/// A change detected between the directory and the last known snapshot.
pub enum DirChange {
    Added(ServerDocument),
    Updated(ServerDocument),
    Removed(ServerId),
}

impl DirChange {
    fn into_event(self) -> PluginProviderEvent {
        match self {
            DirChange::Added(doc) => PluginProviderEvent::AddedDocument(doc),
            DirChange::Updated(doc) => PluginProviderEvent::UpdatedDocument(doc),
            DirChange::Removed(id) => PluginProviderEvent::Removed(id),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ReloadSummary {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

struct Entry {
    text: String,
    config: ServerConfig,
}

pub struct ServerDir {
    dir: PathBuf,
    snapshot: Mutex<HashMap<PathBuf, Entry>>,
    /// Serializes writers against the rescan, so a self-write can never be
    /// observed half-applied and diffed back out.
    write_lock: tokio::sync::Mutex<()>,
}

impl ServerDir {
    /// Opens `<data_dir>/servers/`, migrating a legacy `servers.json` if present.
    pub fn open(data_dir: &Path) -> Self {
        let dir = data_dir.join(SERVERS_SUBDIR);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::error!(dir = %dir.display(), error = %e, "Failed to create servers directory");
        }

        migrate_legacy_store(data_dir, &dir);

        let snapshot: HashMap<PathBuf, Entry> = scan(&dir)
            .map_err(|e| tracing::error!(dir = %dir.display(), error = %e, "Failed to read the servers directory"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(path, entry)| entry.map(|entry| (path, entry)))
            .collect();

        tracing::info!(
            dir = %dir.display(),
            count = snapshot.len(),
            "Loaded API-managed servers"
        );

        Self {
            dir,
            snapshot: Mutex::new(snapshot),
            write_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// The documents to hand the proxy when it activates the provider.
    /// Rescans first: a file dropped in before the provider was wired up gets
    /// no further watcher event to announce it.
    pub async fn initial_documents(&self) -> Vec<ServerDocument> {
        self.diff().await;
        self.documents()
    }

    pub fn documents(&self) -> Vec<ServerDocument> {
        self.snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(|(path, entry)| {
                Some(ServerDocument {
                    id: ServerId::new(file_stem(path)?),
                    toml: entry.text.clone(),
                })
            })
            .collect()
    }

    /// Whether `id` is one this plugin may write, i.e. served by the document
    /// filed under that very name.
    pub fn owns(&self, id: &str) -> bool {
        DocumentId::new(id).is_some_and(|id| self.ownership(&id) == Ownership::Owned)
    }

    pub fn ownership(&self, id: &DocumentId) -> Ownership {
        let path = self.path_of(id);
        let snapshot = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());

        match snapshot.get(&path) {
            Some(entry) if entry.config.effective_id() == id.as_str() => Ownership::Owned,
            Some(_) => Ownership::Shadowed,
            None if snapshot
                .values()
                .any(|entry| entry.config.effective_id() == id.as_str()) =>
            {
                Ownership::Shadowed
            }
            None => Ownership::Absent,
        }
    }

    pub fn document_text(&self, id: &str) -> Option<String> {
        let path = self.path_of(&DocumentId::new(id)?);
        self.snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&path)
            .filter(|entry| entry.config.effective_id() == id)
            .map(|entry| entry.text.clone())
    }

    /// Files a new document as `id`, refusing a name already taken on disk
    /// whatever that file holds.
    pub async fn create(&self, id: &DocumentId, text: String) -> Result<Committed<'_>, StoreError> {
        let config = document_config(id, &text)?;
        let path = self.path_of(id);

        let guard = self.write_lock.lock().await;
        if path.try_exists().unwrap_or(true) {
            return Err(StoreError::Occupied(id.to_string()));
        }
        self.store(path, text, config).await?;

        Ok(Committed {
            id: id.clone(),
            _guard: guard,
        })
    }

    /// Replaces the document filed as `id`.
    pub async fn replace(
        &self,
        id: &DocumentId,
        text: String,
    ) -> Result<Committed<'_>, StoreError> {
        let config = document_config(id, &text)?;
        let path = self.path_of(id);

        let guard = self.write_lock.lock().await;
        self.store(path, text, config).await?;

        Ok(Committed {
            id: id.clone(),
            _guard: guard,
        })
    }

    /// Deletes the document filed under `id`, or `None` when there is none.
    pub async fn remove(&self, id: &DocumentId) -> Result<Option<Committed<'_>>, StoreError> {
        let path = self.path_of(id);
        let guard = self.write_lock.lock().await;

        if !self
            .snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&path)
        {
            return Ok(None);
        }

        if let Err(source) = tokio::fs::remove_file(&path).await
            && source.kind() != std::io::ErrorKind::NotFound
        {
            return Err(StoreError::Io { path, source });
        }

        self.snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&path);

        Ok(Some(Committed {
            id: id.clone(),
            _guard: guard,
        }))
    }

    async fn store(
        &self,
        path: PathBuf,
        text: String,
        config: ServerConfig,
    ) -> Result<(), StoreError> {
        let tmp = path.with_extension("toml.tmp");

        tokio::fs::write(&tmp, text.as_bytes())
            .await
            .map_err(|source| StoreError::Io {
                path: tmp.clone(),
                source,
            })?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;

        self.snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(path, Entry { text, config });

        Ok(())
    }

    /// Rescans the directory and returns the difference against the snapshot,
    /// which is updated in place.
    pub async fn diff(&self) -> Vec<DirChange> {
        let _guard = self.write_lock.lock().await;

        let current = match scan(&self.dir) {
            Ok(current) => current,
            Err(e) => {
                tracing::error!(
                    dir = %self.dir.display(),
                    error = %e,
                    "Failed to read the servers directory, keeping the known servers"
                );
                return Vec::new();
            }
        };
        let mut snapshot = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
        let mut changes = Vec::new();

        for (path, entry) in current
            .iter()
            .filter_map(|(path, entry)| entry.as_ref().map(|entry| (path, entry)))
        {
            let Some(stem) = file_stem(path) else {
                continue;
            };
            let document = ServerDocument {
                id: ServerId::new(stem),
                toml: entry.text.clone(),
            };

            match snapshot.get(path) {
                Some(known) if known.config == entry.config => {}
                Some(_) => changes.push(DirChange::Updated(document)),
                None => changes.push(DirChange::Added(document)),
            }

            snapshot.insert(
                path.clone(),
                Entry {
                    text: entry.text.clone(),
                    config: entry.config.clone(),
                },
            );
        }

        let removed: Vec<PathBuf> = snapshot
            .keys()
            .filter(|path| !current.contains_key(*path))
            .cloned()
            .collect();

        for path in removed {
            snapshot.remove(&path);
            if let Some(stem) = file_stem(&path) {
                changes.push(DirChange::Removed(ServerId::new(stem)));
            }
        }

        changes
    }

    fn path_of(&self, id: &DocumentId) -> PathBuf {
        self.dir.join(format!("{id}.toml"))
    }
}

/// The config the directory holds for a document filed as `id`: the very one
/// a later rescan produces, so a self-write never diffs back out.
///
/// A document identifying as something else is refused: the proxy would route
/// it under a name that is not the one it is filed under.
fn document_config(id: &DocumentId, text: &str) -> Result<ServerConfig, StoreError> {
    let config = parse_document(id.as_str(), text).map_err(StoreError::Document)?;
    let claimed = config.effective_id();
    if claimed != id.as_str() {
        return Err(StoreError::IdMismatch {
            id: id.to_string(),
            claimed,
        });
    }

    Ok(config)
}

/// Parses a server document the same way the proxy does: the file stem
/// supplies the id when the document omits it.
///
/// # Errors
///
/// Returns a human-readable message when the TOML does not parse or the
/// resulting config fails validation.
pub fn parse_document(stem: &str, text: &str) -> Result<ServerConfig, String> {
    let mut config: ServerConfig = toml::from_str(text).map_err(|e| e.to_string())?;

    if config.id.is_none() {
        config.id = Some(stem.to_string());
    }

    infrarust_config::validate_server_config(&config).map_err(|e| e.to_string())?;
    Ok(config)
}

pub fn to_document_text(config: &ServerConfig) -> Result<String, StoreError> {
    toml::to_string_pretty(config).map_err(|source| StoreError::Serialize {
        id: config.effective_id(),
        source,
    })
}

/// Rescans the directory and forwards every change to the proxy, or `None`
/// when the proxy's sender is not connected yet.
///
/// The filesystem watcher and `POST /api/v1/config/reload` both go through
/// here. Nothing is diffed while the sender is missing, so the changes stay
/// pending instead of being consumed by a rescan nobody hears.
pub async fn reload(dir: &ServerDir, sender: &ProviderSenderSlot) -> Option<ReloadSummary> {
    let mut summary = ReloadSummary::default();

    let guard = sender.lock().await;
    let Some(sender) = guard.as_ref() else {
        tracing::warn!("Config provider is not connected yet, skipping rescan");
        return None;
    };

    for change in dir.diff().await {
        match &change {
            DirChange::Added(_) => summary.added += 1,
            DirChange::Updated(_) => summary.updated += 1,
            DirChange::Removed(_) => summary.removed += 1,
        }
        sender.send(change.into_event()).await;
    }

    Some(summary)
}

/// Watches the servers directory and reloads on every debounced burst.
pub async fn watch(
    dir: Arc<ServerDir>,
    sender: Arc<ProviderSenderSlot>,
    shutdown: CancellationToken,
) {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<()>();

    let watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
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
    .and_then(|mut watcher: RecommendedWatcher| {
        watcher.watch(&dir.dir, RecursiveMode::NonRecursive)?;
        Ok(watcher)
    });

    let _watcher = match watcher {
        Ok(watcher) => watcher,
        Err(e) => {
            tracing::error!(
                dir = %dir.dir.display(),
                error = %e,
                "Failed to watch servers directory, only manual reloads will apply"
            );
            return;
        }
    };

    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            recv = notify_rx.recv() => {
                if recv.is_none() {
                    break;
                }
                tokio::time::sleep(DEBOUNCE).await;
                while notify_rx.try_recv().is_ok() {}

                if let Some(summary) = reload(&dir, &sender).await
                    && summary.added + summary.updated + summary.removed > 0
                {
                    tracing::info!(?summary, "Servers directory changed");
                }
            }
        }
    }
}

/// Every `.toml` in the directory, `None` for the ones that do not parse.
///
/// A directory that cannot be read in full is an error rather than an empty
/// listing: reporting it as empty would unregister every server the plugin
/// serves.
fn scan(dir: &Path) -> Result<HashMap<PathBuf, Option<Entry>>, std::io::Error> {
    let mut entries = HashMap::new();

    for read in std::fs::read_dir(dir)? {
        let path = read?.path();
        if path.extension().is_none_or(|ext| ext != "toml") {
            continue;
        }

        let entry = load(&path).map_or_else(
            |e| {
                tracing::warn!(path = %path.display(), error = %e, "Ignoring invalid server file");
                None
            },
            Some,
        );
        entries.insert(path, entry);
    }

    Ok(entries)
}

fn load(path: &Path) -> Result<Entry, String> {
    let stem = file_stem(path).ok_or_else(|| "file name is not valid UTF-8".to_string())?;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let config = parse_document(stem, &text)?;
    Ok(Entry { text, config })
}

fn file_stem(path: &Path) -> Option<&str> {
    path.file_stem().and_then(|stem| stem.to_str())
}

/// Pre-2.0 persistence format: a JSON array in `<data_dir>/servers.json`.
#[derive(Deserialize)]
struct StoredServer {
    id: String,
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default)]
    proxy_mode: String,
    #[serde(default)]
    limbo_handlers: Vec<String>,
}

#[derive(Serialize)]
struct MigratedServer<'a> {
    id: &'a str,
    domains: &'a [String],
    addresses: &'a [String],
    proxy_mode: &'a str,
    limbo_handlers: &'a [String],
}

fn migrate_legacy_store(data_dir: &Path, dir: &Path) {
    let legacy = data_dir.join(LEGACY_STORE);
    if !legacy.exists() {
        return;
    }

    let stored = match std::fs::read_to_string(&legacy)
        .map_err(|e| e.to_string())
        .and_then(|content| {
            serde_json::from_str::<Vec<StoredServer>>(&content).map_err(|e| e.to_string())
        }) {
        Ok(stored) => stored,
        Err(e) => {
            tracing::error!(path = %legacy.display(), error = %e, "Failed to read legacy servers.json");
            return;
        }
    };

    let mut migrated = 0;
    let mut failed = 0;
    for server in &stored {
        let Some(id) = DocumentId::new(&server.id) else {
            tracing::warn!(id = %server.id, "Skipping legacy server with an unusable id");
            failed += 1;
            continue;
        };
        let path = dir.join(format!("{id}.toml"));
        if path.exists() {
            continue;
        }

        let proxy_mode = crate::util::proxy_mode_str(
            crate::util::parse_proxy_mode(&server.proxy_mode)
                .unwrap_or(infrarust_api::services::config_service::ProxyMode::Passthrough),
        );
        let document = match toml::to_string_pretty(&MigratedServer {
            id: &server.id,
            domains: &server.domains,
            addresses: &server.addresses,
            proxy_mode,
            limbo_handlers: &server.limbo_handlers,
        }) {
            Ok(document) => document,
            Err(e) => {
                tracing::warn!(id = %server.id, error = %e, "Skipping unconvertible legacy server");
                failed += 1;
                continue;
            }
        };

        if let Err(e) = parse_document(id.as_str(), &document) {
            tracing::warn!(id = %server.id, error = %e, "Skipping invalid legacy server");
            failed += 1;
            continue;
        }

        if let Err(e) = std::fs::write(&path, document.as_bytes()) {
            tracing::error!(path = %path.display(), error = %e, "Failed to migrate legacy server");
            failed += 1;
            continue;
        }
        migrated += 1;
    }

    if failed > 0 {
        tracing::error!(
            migrated,
            failed,
            path = %legacy.display(),
            "Legacy servers.json was migrated only in part and is kept as is; \
             fix or drop the servers that failed and restart"
        );
        return;
    }

    let done = legacy.with_extension("json.migrated");
    if let Err(e) = std::fs::rename(&legacy, &done) {
        tracing::error!(path = %legacy.display(), error = %e, "Failed to rename legacy servers.json");
        return;
    }

    tracing::info!(count = migrated, path = %done.display(), "Migrated legacy servers.json");
}

#[cfg(test)]
pub(crate) mod test_support {
    //! [`PluginProviderSender`]s standing in for the proxy.

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use infrarust_api::event::BoxFuture;

    use super::{PluginProviderEvent, PluginProviderSender, ServerDir};

    #[derive(Clone, Default)]
    pub(crate) struct RecordingSender(Arc<Mutex<Vec<String>>>);

    impl RecordingSender {
        pub(crate) fn events(&self) -> Vec<String> {
            self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
        }
    }

    impl PluginProviderSender for RecordingSender {
        fn send(&self, event: PluginProviderEvent) -> BoxFuture<'_, bool> {
            let label = match event {
                PluginProviderEvent::AddedDocument(doc) => format!("added:{}", doc.id.as_str()),
                PluginProviderEvent::UpdatedDocument(doc) => format!("updated:{}", doc.id.as_str()),
                PluginProviderEvent::Removed(id) => format!("removed:{}", id.as_str()),
                _ => "other".to_string(),
            };
            self.0.lock().unwrap_or_else(|p| p.into_inner()).push(label);
            Box::pin(async { true })
        }

        fn is_shutdown(&self) -> bool {
            false
        }
    }

    /// Tries to rescan the directory from inside the event the write emits,
    /// reporting whether the write was still holding it.
    pub(crate) struct RescanDuringSend {
        dir: Arc<ServerDir>,
        blocked: Arc<AtomicBool>,
    }

    impl RescanDuringSend {
        pub(crate) fn new(dir: Arc<ServerDir>) -> (Self, Arc<AtomicBool>) {
            let blocked = Arc::new(AtomicBool::new(false));
            (
                Self {
                    dir,
                    blocked: blocked.clone(),
                },
                blocked,
            )
        }
    }

    impl PluginProviderSender for RescanDuringSend {
        fn send(&self, _event: PluginProviderEvent) -> BoxFuture<'_, bool> {
            Box::pin(async move {
                let rescan = tokio::time::timeout(Duration::from_millis(200), self.dir.diff());
                self.blocked.store(rescan.await.is_err(), Ordering::SeqCst);
                true
            })
        }

        fn is_shutdown(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const SURVIVAL: &str = r#"
        domains = ["mc.example.com"]
        addresses = ["10.0.0.1:25565", { address = "10.0.0.2:25565", weight = 3 }]
        balance = "least_conn"
        slow_start = "45s"
        slow_start_aggression = 2.5

        [active_health]
        kind = "status_ping"
        probe_healthy = true

        [motd.online]
        text = "Welcome"
        max_players = 42
    "#;

    fn open() -> (tempfile::TempDir, ServerDir) {
        let root = tempfile::tempdir().unwrap();
        let dir = ServerDir::open(root.path());
        (root, dir)
    }

    fn id(id: &str) -> DocumentId {
        DocumentId::new(id).unwrap()
    }

    /// The API's own write path: serialize what was submitted, then file it.
    async fn store(dir: &ServerDir, id: &str, source: &str) -> String {
        let config = parse_document(id, source).unwrap();
        let text = to_document_text(&config).unwrap();
        dir.create(&DocumentId::new(id).unwrap(), text)
            .await
            .unwrap()
            .id()
            .to_string()
    }

    #[tokio::test]
    async fn write_returns_the_id_and_produces_no_diff() {
        let (_root, dir) = open();

        assert_eq!(store(&dir, "survival", SURVIVAL).await, "survival");
        assert!(dir.diff().await.is_empty());
        assert!(dir.owns("survival"));
    }

    /// A document naming itself through `name` keeps `id` unset, which is the
    /// shape `settle_id` leaves it in. What the store remembers must still be
    /// what a rescan reads back, or every write echoes as an update.
    #[tokio::test]
    async fn write_of_a_name_only_document_produces_no_diff() {
        let (_root, dir) = open();
        let text = "name = \"survival\"\ndomains = [\"mc.example.com\"]\n\
                    addresses = [\"10.0.0.1:25565\"]\n"
            .to_string();

        let committed = dir.create(&id("survival"), text).await.unwrap();
        assert_eq!(committed.id().as_str(), "survival");
        drop(committed);

        let changes: Vec<String> = dir.diff().await.iter().map(describe).collect();
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[tokio::test]
    async fn create_refuses_a_name_another_document_already_uses() {
        let (root, dir) = open();
        let path = root.path().join("servers/foo.toml");
        std::fs::write(
            &path,
            "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
        )
        .unwrap();
        dir.diff().await;

        let text = "id = \"foo\"\ndomains = [\"foo.example.com\"]\n\
                    addresses = [\"10.0.0.1:25565\"]\n"
            .to_string();
        let error = dir.create(&id("foo"), text).await.unwrap_err();

        assert!(matches!(error, StoreError::Occupied(_)), "{error:?}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("bar"));
    }

    #[tokio::test]
    async fn a_document_filed_under_another_name_is_not_owned() {
        let (root, dir) = open();
        std::fs::write(
            root.path().join("servers/foo.toml"),
            "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
        )
        .unwrap();
        dir.diff().await;

        assert_eq!(dir.ownership(&id("foo")), Ownership::Shadowed);
        assert_eq!(dir.ownership(&id("bar")), Ownership::Shadowed);
        assert!(!dir.owns("foo"));
        assert!(!dir.owns("bar"));
    }

    #[tokio::test]
    async fn a_document_cannot_be_filed_under_a_name_it_does_not_claim() {
        let (_root, dir) = open();
        let text = "name = \"bar\"\ndomains = [\"bar.example.com\"]\n\
                    addresses = [\"10.0.0.9:25565\"]\n"
            .to_string();

        let error = dir.create(&id("foo"), text).await.unwrap_err();

        assert!(matches!(error, StoreError::IdMismatch { .. }), "{error:?}");
        assert!(!_root.path().join("servers/foo.toml").exists());
    }

    #[tokio::test]
    async fn stored_document_keeps_every_field() {
        let (root, dir) = open();
        store(&dir, "survival", SURVIVAL).await;

        let reopened = ServerDir::open(root.path());
        let document = reopened.document_text("survival").unwrap();
        let config = parse_document("survival", &document).unwrap();

        assert_eq!(config.balance, infrarust_config::BalanceStrategy::LeastConn);
        assert_eq!(config.slow_start, Some(Duration::from_secs(45)));
        assert!((config.slow_start_aggression - 2.5).abs() < f64::EPSILON);
        assert_eq!(config.addresses[1].weight, 3);
        assert_eq!(
            config.active_health.as_ref().unwrap().kind,
            infrarust_config::ProbeKind::StatusPing
        );
        assert!(config.active_health.as_ref().unwrap().probe_healthy);
        assert_eq!(config.motd.online.as_ref().unwrap().max_players, Some(42));
        assert!(reopened.diff().await.is_empty());
    }

    #[tokio::test]
    async fn diff_reports_added_updated_and_removed() {
        let (root, dir) = open();
        let path = root.path().join("servers/lobby.toml");

        std::fs::write(
            &path,
            "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
        )
        .unwrap();
        let changes = dir.diff().await;
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], DirChange::Added(doc) if doc.id.as_str() == "lobby"));

        std::fs::write(
            &path,
            "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:2\"]\n",
        )
        .unwrap();
        let changes = dir.diff().await;
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], DirChange::Updated(_)));

        assert!(dir.diff().await.is_empty());

        std::fs::remove_file(&path).unwrap();
        let changes = dir.diff().await;
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], DirChange::Removed(id) if id.as_str() == "lobby"));
    }

    /// A directory that cannot be listed says nothing about its servers, so it
    /// must not unregister them.
    #[tokio::test]
    async fn an_unreadable_directory_removes_nothing() {
        let (root, dir) = open();
        store(&dir, "survival", SURVIVAL).await;

        std::fs::remove_dir_all(root.path().join("servers")).unwrap();

        let changes: Vec<String> = dir.diff().await.iter().map(describe).collect();
        assert!(changes.is_empty(), "{changes:?}");
        assert!(dir.owns("survival"));
    }

    #[tokio::test]
    async fn invalid_file_keeps_the_previous_version() {
        let (root, dir) = open();
        let path = root.path().join("servers/lobby.toml");

        std::fs::write(
            &path,
            "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
        )
        .unwrap();
        assert_eq!(dir.diff().await.len(), 1);

        std::fs::write(&path, "addresses = [").unwrap();
        assert!(dir.diff().await.is_empty());
        assert!(dir.owns("lobby"));
    }

    #[tokio::test]
    async fn remove_deletes_the_file() {
        let (root, dir) = open();
        store(&dir, "survival", SURVIVAL).await;

        let removed = dir.remove(&id("survival")).await.unwrap();
        assert_eq!(
            removed.map(|removed| removed.id().to_string()).as_deref(),
            Some("survival")
        );
        assert!(!root.path().join("servers/survival.toml").exists());
        assert!(dir.remove(&id("survival")).await.unwrap().is_none());
        assert!(dir.diff().await.is_empty());
    }

    /// A file that appears before the proxy connects its sender gets no second
    /// watcher event, so the initial hand-over has to rescan.
    #[tokio::test]
    async fn initial_documents_see_files_written_after_open() {
        let (root, dir) = open();
        std::fs::write(
            root.path().join("servers/lobby.toml"),
            "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
        )
        .unwrap();

        let documents = dir.initial_documents().await;
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].id.as_str(), "lobby");
    }

    #[tokio::test]
    async fn reload_without_a_sender_reports_nothing_and_keeps_the_change_pending() {
        let (root, dir) = open();
        let slot: ProviderSenderSlot = tokio::sync::Mutex::new(None);
        std::fs::write(
            root.path().join("servers/lobby.toml"),
            "domains = [\"a.example.com\"]\naddresses = [\"10.0.0.1:1\"]\n",
        )
        .unwrap();

        assert!(reload(&dir, &slot).await.is_none());

        let sender = test_support::RecordingSender::default();
        *slot.lock().await = Some(Box::new(sender.clone()));
        let summary = reload(&dir, &slot).await.unwrap();

        assert_eq!(summary.added, 1);
        assert_eq!(sender.events(), vec!["added:lobby".to_string()]);
    }

    #[test]
    fn document_ids_round_trip_through_a_file_name() {
        let (root, dir) = open();
        let servers = root.path().join("servers");

        for candidate in ["lobby", "mc.lobby", "a-b_c", "9"] {
            let document = id(candidate);
            let path = dir.path_of(&document);
            assert_eq!(path.parent(), Some(servers.as_path()), "{candidate}");
            assert_eq!(file_stem(&path), Some(candidate), "{candidate}");
        }
    }

    #[test]
    fn document_ids_reject_anything_a_file_name_cannot_carry() {
        for candidate in [
            "", "..", ".", "../x", "a/b", "C:\\evil", ".hidden", "a..b", "UPPER", "a b", "e\u{301}",
        ] {
            assert_eq!(DocumentId::new(candidate), None, "{candidate}");
        }
        assert_eq!(DocumentId::new(&"a".repeat(MAX_ID_LEN + 1)), None);
    }

    #[test]
    fn legacy_store_is_migrated() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(LEGACY_STORE),
            r#"[{"id":"lobby","domains":["lobby.example.com"],
                 "addresses":["127.0.0.1:25565"],"proxy_mode":"zerocopy",
                 "limbo_handlers":["queue"]}]"#,
        )
        .unwrap();

        let dir = ServerDir::open(root.path());

        assert!(root.path().join("servers.json.migrated").exists());
        assert!(!root.path().join(LEGACY_STORE).exists());

        let config = parse_document(
            "lobby",
            &dir.document_text("lobby").expect("migrated document"),
        )
        .unwrap();
        assert_eq!(config.proxy_mode, infrarust_config::ProxyMode::ZeroCopy);
        assert_eq!(config.domains, vec!["lobby.example.com".to_string()]);
        assert_eq!(config.limbo_handlers, vec!["queue".to_string()]);
    }

    /// A half-migrated store that is renamed away leaves the operator nothing
    /// to retry from.
    #[test]
    fn a_partly_migrated_legacy_store_is_kept() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(LEGACY_STORE),
            r#"[{"id":"lobby","domains":["lobby.example.com"],
                 "addresses":["127.0.0.1:25565"],"proxy_mode":"passthrough"},
                {"id":"My Server","domains":["mine.example.com"],
                 "addresses":["127.0.0.1:25566"],"proxy_mode":"passthrough"}]"#,
        )
        .unwrap();

        let dir = ServerDir::open(root.path());

        assert!(dir.owns("lobby"));
        assert!(root.path().join(LEGACY_STORE).exists());
        assert!(!root.path().join("servers.json.migrated").exists());
    }

    fn describe(change: &DirChange) -> String {
        match change {
            DirChange::Added(doc) => format!("added:{}", doc.id.as_str()),
            DirChange::Updated(doc) => format!("updated:{}", doc.id.as_str()),
            DirChange::Removed(id) => format!("removed:{}", id.as_str()),
        }
    }
}
