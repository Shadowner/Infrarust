//! `ProviderRegistry` — orchestrates config providers and feeds the `DomainRouter`.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_api::events::proxy::ConfigReloadEvent;
use infrarust_api::types::ServerId;
use infrarust_config::{ForwardingMode, ServerConfig};

use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::routing::DomainRouter;
use crate::status::{FaviconCache, StatusCache};

use super::{ConfigProvider, ProviderChange, ProviderConfig, ProviderEvent, ProviderId};

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
    default_forwarding: ForwardingMode,
}

impl ProviderRegistry {
    pub fn new(
        domain_router: Arc<DomainRouter>,
        event_bus: Arc<EventBusImpl>,
        status_cache: Arc<StatusCache>,
        favicon_cache: Arc<FaviconCache>,
        shutdown: CancellationToken,
        default_forwarding: ForwardingMode,
    ) -> Self {
        Self {
            providers: Vec::new(),
            domain_router,
            event_bus,
            status_cache,
            favicon_cache,
            shutdown,
            default_forwarding,
        }
    }

    /// Registers a provider. Must be called before `start()`.
    pub fn add_provider(&mut self, provider: Box<dyn ConfigProvider>) {
        self.providers.push(provider);
    }

    /// Starts all providers: loads initial configs and spawns watchers.
    ///
    /// Consumes `self` to transfer ownership of providers to spawned tasks.
    /// Returns the `JoinHandle` of the event loop task and a sender that
    /// can be used to inject events from plugin config providers.
    ///
    /// # Errors
    /// Returns `CoreError` if any provider fails to load initial configs fatally.
    pub async fn start(self) -> Result<(JoinHandle<()>, mpsc::Sender<ProviderEvent>), CoreError> {
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
                        if accepted(&pc, &self.default_forwarding) {
                            self.domain_router.add(pc.id, pc.config);
                        }
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

        let plugin_tx = tx.clone();
        drop(tx);

        // Phase 3: spawn the event loop
        let shutdown_token = self.shutdown.clone();
        let handle = tokio::spawn(async move {
            event_loop(
                rx,
                self.domain_router,
                self.event_bus,
                self.status_cache,
                self.favicon_cache,
                self.shutdown,
                self.default_forwarding,
            )
            .await;
            if !shutdown_token.is_cancelled() {
                tracing::error!(
                    "provider event loop exited unexpectedly, configuration hot-reload is no longer active"
                );
            }
        });

        Ok((handle, plugin_tx))
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
    default_forwarding: ForwardingMode,
) {
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                tracing::debug!("provider registry shutting down");
                break;
            }
            event = rx.recv() => {
                let Some(event) = event else {
                    tracing::debug!("all provider senders dropped, event loop exiting");
                    break;
                };
                let reloads = apply(&router, &default_forwarding, event);
                if !reloads.is_empty() {
                    on_config_change(&router, &status_cache, &favicon_cache).await;
                    for reload in reloads {
                        event_bus.post(reload);
                    }
                }
            }
        }
    }
}

fn accepted(pc: &ProviderConfig, default_forwarding: &ForwardingMode) -> bool {
    match infrarust_config::validate_server_forwarding(&pc.config, default_forwarding.clone()) {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(id = %pc.id, %error, "server config rejected");
            false
        }
    }
}

fn apply(
    router: &DomainRouter,
    default_forwarding: &ForwardingMode,
    event: ProviderEvent,
) -> Vec<ConfigReloadEvent> {
    let changes = event.into_changes();

    let mut before: Vec<(ProviderId, Option<Arc<ServerConfig>>)> = Vec::new();
    for change in changes {
        let id = match &change {
            ProviderChange::Added(pc) | ProviderChange::Updated(pc) => {
                if !accepted(pc, default_forwarding) {
                    continue;
                }
                pc.id.clone()
            }
            ProviderChange::Removed(id) => id.clone(),
        };
        if !before.iter().any(|(seen, _)| *seen == id) {
            let previous = router.get(&id);
            before.push((id, previous));
        }
        match change {
            ProviderChange::Added(pc) => {
                tracing::info!(id = %pc.id, "config added by provider");
                router.add(pc.id, pc.config);
            }
            ProviderChange::Updated(pc) => {
                tracing::info!(id = %pc.id, "config updated by provider");
                router.update(pc.id, pc.config);
            }
            ProviderChange::Removed(id) => {
                tracing::info!(id = %id, "config removed by provider");
                router.remove(&id);
            }
        }
    }

    let mut reloads: Vec<ConfigReloadEvent> = Vec::new();
    for (id, previous) in before {
        let position = reloads
            .iter()
            .position(|reload| reload.provider == id.provider_type)
            .unwrap_or_else(|| {
                reloads.push(ConfigReloadEvent::new(
                    id.provider_type.clone(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ));
                reloads.len() - 1
            });
        let reload = &mut reloads[position];
        match (previous, router.get(&id)) {
            (None, Some(now)) => reload.added.push(ServerId::new(now.effective_id())),
            (Some(was), None) => reload.removed.push(ServerId::new(was.effective_id())),
            (Some(was), Some(now)) if was.effective_id() != now.effective_id() => {
                reload.removed.push(ServerId::new(was.effective_id()));
                reload.added.push(ServerId::new(now.effective_id()));
            }
            (Some(was), Some(now)) if was != now => {
                reload.updated.push(ServerId::new(now.effective_id()));
            }
            _ => {}
        }
    }
    reloads.retain(|reload| !reload.is_empty());
    for reload in &mut reloads {
        for ids in [&mut reload.added, &mut reload.removed, &mut reload.updated] {
            ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            ids.dedup();
        }
    }
    reloads
}

async fn on_config_change(
    router: &DomainRouter,
    status_cache: &StatusCache,
    favicon_cache: &FaviconCache,
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn passthrough(name: &str) -> ProviderConfig {
        ProviderConfig {
            id: ProviderId::file(name),
            config: toml::from_str(&format!(
                "name = \"{name}\"\ndomains = [\"{name}.test\"]\naddresses = [\"10.0.0.1:25565\"]\n"
            ))
            .unwrap(),
        }
    }

    #[test]
    fn a_forwarding_server_under_a_velocity_default_is_not_published() {
        let router = DomainRouter::new();
        let lobby = passthrough("lobby");
        let id = lobby.id.clone();

        let reloads = apply(
            &router,
            &ForwardingMode::Velocity,
            ProviderEvent::Added(lobby),
        );

        assert!(
            router.get(&id).is_none(),
            "the rejected server must not be routed"
        );
        assert!(reloads.is_empty());
    }

    #[test]
    fn a_rejected_update_keeps_the_previous_config() {
        let router = DomainRouter::new();
        let lobby = passthrough("lobby");
        let id = lobby.id.clone();
        apply(&router, &ForwardingMode::None, ProviderEvent::Added(lobby));

        let mut broken = passthrough("lobby");
        broken.config.forwarding_mode = Some(ForwardingMode::Velocity);
        apply(
            &router,
            &ForwardingMode::None,
            ProviderEvent::Updated(broken),
        );

        let kept = router.get(&id).expect("the previous config stays routed");
        assert_eq!(kept.forwarding_mode, None);
    }
}
