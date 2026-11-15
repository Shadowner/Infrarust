use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use infrarust_api::provider::{PluginProviderEvent, ServerDocument};
use infrarust_api::types::ServerId;
use infrarust_config::ServerConfig;

use super::legacy::migrate_legacy_store;

const SERVERS_SUBDIR: &str = "servers";

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

impl From<crate::fs::WriteError> for StoreError {
    fn from(error: crate::fs::WriteError) -> Self {
        StoreError::Io {
            path: error.path,
            source: error.source,
        }
    }
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

pub struct WriteLock<'a> {
    dir: &'a ServerDir,
    guard: tokio::sync::MutexGuard<'a, ()>,
}

impl<'a> WriteLock<'a> {
    /// Replaces the document filed as `id`.
    pub async fn replace(self, id: &DocumentId, text: String) -> Result<Committed<'a>, StoreError> {
        let config = document_config(id, &text)?;
        let path = self.dir.path_of(id);
        self.dir.store(path, text, config).await?;

        Ok(Committed {
            id: id.clone(),
            _guard: self.guard,
        })
    }

    /// Deletes the document filed under `id`, or `None` when there is none.
    pub async fn remove(self, id: &DocumentId) -> Result<Option<Committed<'a>>, StoreError> {
        let path = self.dir.path_of(id);

        if !self.dir.snapshot().contains_key(&path) {
            return Ok(None);
        }

        if let Err(source) = tokio::fs::remove_file(&path).await
            && source.kind() != std::io::ErrorKind::NotFound
        {
            return Err(StoreError::Io { path, source });
        }

        self.dir.snapshot().remove(&path);

        Ok(Some(Committed {
            id: id.clone(),
            _guard: self.guard,
        }))
    }
}

/// A change detected between the directory and the last known snapshot.
pub enum DirChange {
    Added(ServerDocument),
    Updated(ServerDocument),
    Removed(ServerId),
}

impl DirChange {
    pub(super) fn into_event(self) -> PluginProviderEvent {
        match self {
            DirChange::Added(doc) => PluginProviderEvent::Added(doc),
            DirChange::Updated(doc) => PluginProviderEvent::Updated(doc),
            DirChange::Removed(id) => PluginProviderEvent::Removed(id),
        }
    }
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
    pub fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let dir = data_dir.join(SERVERS_SUBDIR);
        std::fs::create_dir_all(&dir).map_err(|source| StoreError::Io {
            path: dir.clone(),
            source,
        })?;

        migrate_legacy_store(data_dir, &dir);

        let snapshot: HashMap<PathBuf, Entry> = scan(&dir)
            .map_err(|source| StoreError::Io {
                path: dir.clone(),
                source,
            })?
            .into_iter()
            .filter_map(|(path, entry)| entry.map(|entry| (path, entry)))
            .collect();

        tracing::info!(
            dir = %dir.display(),
            count = snapshot.len(),
            "Loaded API-managed servers"
        );

        Ok(Self {
            dir,
            snapshot: Mutex::new(snapshot),
            write_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// The documents to hand the proxy when it activates the provider.
    /// Rescans first: a file dropped in before the provider was wired up gets
    /// no further watcher event to announce it.
    pub async fn initial_documents(&self) -> Vec<ServerDocument> {
        self.diff().await;
        self.documents()
    }

    pub fn documents(&self) -> Vec<ServerDocument> {
        self.snapshot()
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
        let snapshot = self.snapshot();

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
        self.snapshot()
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

    /// Takes the write lock, so the caller can check a document and then
    /// write it without another writer slipping in between.
    pub async fn lock_writes(&self) -> WriteLock<'_> {
        WriteLock {
            dir: self,
            guard: self.write_lock.lock().await,
        }
    }

    async fn store(
        &self,
        path: PathBuf,
        text: String,
        config: ServerConfig,
    ) -> Result<(), StoreError> {
        crate::fs::write_atomic(&path, text.as_bytes()).await?;

        self.snapshot().insert(path, Entry { text, config });

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
        let mut snapshot = self.snapshot();
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

    pub(super) fn path(&self) -> &Path {
        &self.dir
    }

    pub(super) fn path_of(&self, id: &DocumentId) -> PathBuf {
        self.dir.join(format!("{id}.toml"))
    }

    fn snapshot(&self) -> MutexGuard<'_, HashMap<PathBuf, Entry>> {
        self.snapshot.lock().unwrap_or_else(|p| p.into_inner())
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

    // The proxy reports the warnings when it applies the document.
    let _ = infrarust_config::validate_server_config(&config).map_err(|e| e.to_string())?;
    Ok(config)
}

pub fn to_document_text(config: &ServerConfig) -> Result<String, StoreError> {
    toml::to_string_pretty(config).map_err(|source| StoreError::Serialize {
        id: config.effective_id(),
        source,
    })
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

pub(super) fn file_stem(path: &Path) -> Option<&str> {
    path.file_stem().and_then(|stem| stem.to_str())
}
