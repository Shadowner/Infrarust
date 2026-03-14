use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_config::{DomainIndex, ServerConfig};

use crate::provider::ConfigChange;

/// Runs the config hot-reload loop.
///
/// Listens for `ConfigChange` events and atomically swaps
/// the domain index and config map via `ArcSwap`.
#[allow(clippy::implicit_hasher)] // ArcSwap<HashMap> is the canonical type used across the crate
pub async fn run_config_watcher(
    mut rx: mpsc::Receiver<ConfigChange>,
    domain_index: Arc<ArcSwap<DomainIndex>>,
    configs: Arc<ArcSwap<HashMap<String, Arc<ServerConfig>>>>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            change = rx.recv() => {
                match change {
                    Some(ConfigChange::FullReload(new_configs)) => {
                        let index = DomainIndex::build(&new_configs);
                        let map: HashMap<String, Arc<ServerConfig>> = new_configs
                            .into_iter()
                            .map(|c| (c.effective_id(), Arc::new(c)))
                            .collect();
                        let count = map.len();
                        domain_index.store(Arc::new(index));
                        configs.store(Arc::new(map));
                        tracing::info!(servers = count, "config reloaded");
                    }
                    None => {
                        tracing::debug!("config watcher channel closed");
                        break;
                    }
                }
            }
            () = shutdown.cancelled() => {
                tracing::debug!("config watcher shutting down");
                break;
            }
        }
    }
}
