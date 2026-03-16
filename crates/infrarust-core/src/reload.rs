use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_api::events::proxy::ConfigReloadEvent;
use infrarust_config::{DomainIndex, ServerConfig};

use crate::event_bus::EventBusImpl;
use crate::provider::ConfigChange;
use crate::status::{FaviconCache, StatusCache};

/// Runs the config hot-reload loop.
///
/// Listens for `ConfigChange` events and atomically swaps
/// the domain index and config map via `ArcSwap`. Also invalidates
/// the status cache and reloads favicons on config change.
#[allow(clippy::implicit_hasher)] // ArcSwap<HashMap> is the canonical type used across the crate
pub async fn run_config_watcher(
    mut rx: mpsc::Receiver<ConfigChange>,
    domain_index: Arc<ArcSwap<DomainIndex>>,
    configs: Arc<ArcSwap<HashMap<String, Arc<ServerConfig>>>>,
    status_cache: Arc<StatusCache>,
    favicon_cache: Arc<FaviconCache>,
    event_bus: Arc<EventBusImpl>,
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

                        // Invalidate status cache — server addresses may have changed
                        status_cache.invalidate_all();

                        // Reload favicons for new/changed server configs
                        let favicon_configs: Vec<(String, Arc<ServerConfig>)> = configs
                            .load()
                            .iter()
                            .map(|(id, cfg)| (id.clone(), Arc::clone(cfg)))
                            .collect();
                        if let Err(e) = favicon_cache.reload(&favicon_configs, None).await {
                            tracing::warn!(error = %e, "failed to reload favicons");
                        }

                        // Fire ConfigReloadEvent for plugins
                        event_bus.fire_and_forget_arc(ConfigReloadEvent);

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
