//! `ProviderRegistry` — orchestrates config providers and feeds the `DomainRouter`.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_api::events::proxy::ConfigReloadEvent;
use infrarust_config::ServerConfig;

use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::routing::DomainRouter;
use crate::status::{FaviconCache, StatusCache};

use super::{ConfigProvider, ProviderEvent};

/// Orchestrates config providers, feeding their events into the `DomainRouter`.
///
/// The registry:
/// 1. Calls `load_initial()` on each provider and populates the router.
/// 2. Spawns a watch task per provider that sends `ProviderEvent`s into a
///    unified bounded channel.
/// 3. Runs an event loop that updates the router, invalidates caches, and
///    fires `ConfigReloadEvent`.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn ConfigProvider>>,
    domain_router: Arc<DomainRouter>,
    event_bus: Arc<EventBusImpl>,
    status_cache: Arc<StatusCache>,
    favicon_cache: Arc<FaviconCache>,
    shutdown: CancellationToken,
}

impl ProviderRegistry {
    pub fn new(
        domain_router: Arc<DomainRouter>,
        event_bus: Arc<EventBusImpl>,
        status_cache: Arc<StatusCache>,
        favicon_cache: Arc<FaviconCache>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            providers: Vec::new(),
            domain_router,
            event_bus,
            status_cache,
            favicon_cache,
            shutdown,
        }
    }

    /// Registers a provider. Must be called before `start()`.
    pub fn add_provider(&mut self, provider: Box<dyn ConfigProvider>) {
        self.providers.push(provider);
    }

    /// Starts all providers: loads initial configs and spawns watchers.
    ///
    /// Consumes `self` to transfer ownership of providers to spawned tasks.
    /// Returns the `JoinHandle` of the event loop task.
    ///
    /// # Errors
    /// Returns `CoreError` if any provider fails to load initial configs fatally.
    pub async fn start(self) -> Result<JoinHandle<()>, CoreError> {
        let (tx, rx) = mpsc::channel::<ProviderEvent>(256);

        // Phase 1: load initial configs from each provider
        for provider in &self.providers {
            match provider.load_initial().await {
                Ok(configs) => {
                    let count = configs.len();
                    let server_configs: Vec<_> =
                        configs.iter().map(|pc| &pc.config).cloned().collect();
                    if let Err(e) = infrarust_config::validate_server_configs(&server_configs) {
                        tracing::warn!(
                            provider = provider.provider_type(),
                            error = %e,
                            "duplicate server ID detected"
                        );
                    }
                    for pc in configs {
                        self.domain_router.add(pc.id, pc.config);
                    }
                    tracing::info!(
                        provider = provider.provider_type(),
                        count,
                        "provider loaded initial configs"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        provider = provider.provider_type(),
                        error = %e,
                        "provider failed to load initial configs, skipping"
                    );
                }
            }
        }

        // Phase 2: spawn watch tasks
        for provider in self.providers {
            let sender = tx.clone();
            let shutdown = self.shutdown.clone();
            let provider_type = provider.provider_type().to_string();

            tokio::spawn(async move {
                if let Err(e) = provider.watch(sender, shutdown).await {
                    tracing::warn!(
                        provider = %provider_type,
                        error = %e,
                        "provider watch task exited with error"
                    );
                }
            });
        }

        // Drop our copy of the sender so the channel closes when all
        // provider watch tasks have exited.
        drop(tx);

        // Phase 3: spawn the event loop
        let handle = tokio::spawn(event_loop(
            rx,
            self.domain_router,
            self.event_bus,
            self.status_cache,
            self.favicon_cache,
            self.shutdown,
        ));

        Ok(handle)
    }
}

/// Receives `ProviderEvent`s and updates the router + caches.
async fn event_loop(
    mut rx: mpsc::Receiver<ProviderEvent>,
    router: Arc<DomainRouter>,
    event_bus: Arc<EventBusImpl>,
    status_cache: Arc<StatusCache>,
    favicon_cache: Arc<FaviconCache>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                tracing::debug!("provider registry shutting down");
                break;
            }
            event = rx.recv() => {
                match event {
                    Some(ProviderEvent::Added(pc)) => {
                        tracing::info!(id = %pc.id, "config added by provider");
                        router.add(pc.id, pc.config);
                        on_config_change(&router, &status_cache, &favicon_cache, &event_bus).await;
                    }
                    Some(ProviderEvent::Updated(pc)) => {
                        tracing::info!(id = %pc.id, "config updated by provider");
                        router.update(pc.id, pc.config);
                        on_config_change(&router, &status_cache, &favicon_cache, &event_bus).await;
                    }
                    Some(ProviderEvent::Removed(id)) => {
                        tracing::info!(id = %id, "config removed by provider");
                        router.remove(&id);
                        on_config_change(&router, &status_cache, &favicon_cache, &event_bus).await;
                    }
                    None => {
                        tracing::debug!("all provider senders dropped, event loop exiting");
                        break;
                    }
                }
            }
        }
    }
}

/// Common post-change handler: invalidate caches, reload favicons, fire event.
async fn on_config_change(
    router: &DomainRouter,
    status_cache: &StatusCache,
    favicon_cache: &FaviconCache,
    event_bus: &Arc<EventBusImpl>,
) {
    status_cache.invalidate_all();

    let favicon_configs: Vec<(String, Arc<ServerConfig>)> = router
        .list_all()
        .into_iter()
        .map(|(_pid, cfg)| (cfg.effective_id(), cfg))
        .collect();
    if let Err(e) = favicon_cache.reload(&favicon_configs, None).await {
        tracing::warn!(error = %e, "failed to reload favicons after config change");
    }

    event_bus.fire_and_forget_arc(ConfigReloadEvent);
}
