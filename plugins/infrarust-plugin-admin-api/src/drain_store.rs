//! `<data_dir>/drained.json` — operator drain intent, kept across restarts.
//!
//! Drain state itself lives in the proxy's in-memory health table, which is
//! rebuilt from scratch on every start. An operator who drained a backend for
//! maintenance expects it to still be drained after a restart, so the plugin
//! keeps its own copy and replays it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::services::load_balancer::LoadBalancerService;
use infrarust_api::types::{ServerAddress, ServerId};
use tokio_util::sync::CancellationToken;

use crate::util::{format_address, parse_address};

const STORE_FILE: &str = "drained.json";

/// A drained backend may belong to a server whose provider has not published
/// its config yet, so reapplication is retried for a short while.
const REAPPLY_RETRY_DELAY: Duration = Duration::from_secs(2);
const REAPPLY_ATTEMPTS: usize = 5;

type Drained = BTreeMap<String, BTreeSet<String>>;

pub struct DrainStore {
    path: PathBuf,
    drained: Mutex<Drained>,
}

/// Rewrites every key the way [`format_address`] renders it, so that two
/// spellings of one address cannot end up as two entries. Entries written by
/// an older build, or hand-edited, go through this on the way in.
fn canonicalize(raw: Drained) -> Drained {
    raw.into_iter()
        .map(|(server, addresses)| {
            let addresses = addresses
                .iter()
                .filter_map(|address| match parse_address(address) {
                    Ok(parsed) => Some(format_address(&parsed)),
                    Err(_) => {
                        tracing::error!(server = %server, address = %address, "Dropping unparsable drain entry");
                        None
                    }
                })
                .collect();
            (server, addresses)
        })
        .filter(|(_, addresses): &(String, BTreeSet<String>)| !addresses.is_empty())
        .collect()
}

impl DrainStore {
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join(STORE_FILE);
        let drained = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::error!(path = %path.display(), error = %e, "Ignoring unreadable drain store");
                Drained::new()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Drained::new(),
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "Failed to read drain store");
                Drained::new()
            }
        };

        Self {
            path,
            drained: Mutex::new(canonicalize(drained)),
        }
    }

    /// Records the intent and persists it. A write failure is logged, never
    /// propagated: the live drain already took effect.
    pub async fn set(&self, server: &str, address: &ServerAddress, drained: bool) {
        let key = format_address(address);
        let snapshot = {
            let mut entries = self.drained.lock().unwrap_or_else(|p| p.into_inner());
            if drained {
                entries.entry(server.to_string()).or_default().insert(key);
            } else if let Some(addresses) = entries.get_mut(server) {
                addresses.remove(&key);
                if addresses.is_empty() {
                    entries.remove(server);
                }
            }
            serde_json::to_vec_pretty(&*entries)
        };

        match snapshot {
            Ok(bytes) => {
                if let Err(e) = tokio::fs::write(&self.path, bytes).await {
                    tracing::error!(path = %self.path.display(), error = %e, "Failed to persist drain store");
                }
            }
            Err(e) => tracing::error!(error = %e, "Failed to serialize drain store"),
        }
    }

    pub fn entries(&self) -> Vec<(String, ServerAddress)> {
        self.drained
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .flat_map(|(server, addresses)| {
                addresses.iter().filter_map(move |address| {
                    parse_address(address)
                        .ok()
                        .map(|parsed| (server.clone(), parsed))
                })
            })
            .collect()
    }
}

/// Replays the persisted drains onto the proxy, retrying the ones whose
/// server is not routable yet.
pub async fn reapply(
    store: Arc<DrainStore>,
    load_balancer: Arc<dyn LoadBalancerService>,
    shutdown: CancellationToken,
) {
    let mut pending = store.entries();

    for _ in 0..REAPPLY_ATTEMPTS {
        pending.retain(|(server, address)| {
            load_balancer
                .set_drained(&ServerId::new(server.clone()), address, true)
                .is_err()
        });
        if pending.is_empty() {
            return;
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(REAPPLY_RETRY_DELAY) => {}
        }
    }

    for (server, address) in &pending {
        tracing::warn!(
            server = %server,
            address = %address,
            "Drained backend is in no server config, drain not reapplied"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(raw: &str) -> ServerAddress {
        parse_address(raw).unwrap()
    }

    #[tokio::test]
    async fn drains_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = DrainStore::open(dir.path());
        store.set("lobby", &addr("10.0.0.1:25565"), true).await;
        store.set("lobby", &addr("10.0.0.2:25565"), true).await;
        store.set("lobby", &addr("10.0.0.1:25565"), false).await;

        let reopened = DrainStore::open(dir.path());
        assert_eq!(
            reopened.entries(),
            vec![("lobby".to_string(), addr("10.0.0.2:25565"))]
        );
    }

    #[tokio::test]
    async fn clearing_the_last_address_drops_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let store = DrainStore::open(dir.path());
        store.set("lobby", &addr("10.0.0.1:25565"), true).await;
        store.set("lobby", &addr("10.0.0.1:25565"), false).await;

        assert!(DrainStore::open(dir.path()).entries().is_empty());
    }

    #[tokio::test]
    async fn another_spelling_of_a_drained_address_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = DrainStore::open(dir.path());
        store.set("lobby", &addr("[::1]:25565"), true).await;
        store.set("lobby", &addr("::1:25565"), false).await;

        assert!(DrainStore::open(dir.path()).entries().is_empty());
    }

    #[tokio::test]
    async fn an_entry_written_by_an_older_build_can_be_cleared() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(STORE_FILE),
            br#"{"lobby": ["::1:25565", "nonsense"]}"#,
        )
        .unwrap();

        let store = DrainStore::open(dir.path());
        assert_eq!(
            store.entries(),
            vec![("lobby".to_string(), addr("[::1]:25565"))],
            "unparsable entries are dropped, the rest are readable"
        );

        store.set("lobby", &addr("[::1]:25565"), false).await;
        assert!(store.entries().is_empty());
    }

    #[test]
    fn a_corrupt_store_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STORE_FILE), b"{ not json").unwrap();
        assert!(DrainStore::open(dir.path()).entries().is_empty());
    }
}
