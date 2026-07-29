//! Active health probing.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use infrarust_config::{ActiveHealthConfig, ProbeKind, ProxyConfig, ServerAddress, ServerConfig};
use infrarust_protocol::registry::PacketRegistry;

use super::{BackendHealthView, BackendState, PassiveBackendHealth};
use crate::routing::DomainRouter;
use crate::status::STATUS_PROTOCOL_VERSION;

pub struct ActiveHealthProber {
    router: Arc<DomainRouter>,
    health: Arc<PassiveBackendHealth>,
    registry: Arc<PacketRegistry>,
    defaults: ActiveHealthConfig,
}

/// One address to probe, with the settings of the server it belongs to.
struct Target {
    address: ServerAddress,
    kind: ProbeKind,
    timeout: Duration,
}

impl ActiveHealthProber {
    pub fn new(
        router: Arc<DomainRouter>,
        health: Arc<PassiveBackendHealth>,
        registry: Arc<PacketRegistry>,
        config: &ProxyConfig,
    ) -> Self {
        Self {
            router,
            health,
            registry,
            defaults: config.active_health.clone(),
        }
    }

    pub fn spawn(self: Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.defaults.unhealthy_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last_full_sweep: Option<Instant> = None;

            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = ticker.tick() => {}
                }

                let full = self.defaults.probe_healthy
                    && last_full_sweep.is_none_or(|at| at.elapsed() >= self.defaults.interval);
                if full {
                    last_full_sweep = Some(Instant::now());
                }
                Arc::clone(&self).sweep(full).await;
            }
        })
    }

    async fn sweep(self: Arc<Self>, include_healthy: bool) {
        let mut known = HashSet::new();
        let mut targets = Vec::new();

        for (_, config) in self.router.list_all() {
            let settings = self.settings_for(&config);
            for weighted in &config.addresses {
                known.insert(weighted.address.clone());
                if !settings.enabled {
                    continue;
                }
                let healthy = self.health.snapshot(&weighted.address).state == BackendState::Healthy;
                let probe = if healthy {
                    include_healthy && settings.probe_healthy
                } else {
                    self.health.claim_probe(&weighted.address)
                };
                if probe {
                    targets.push(Target {
                        address: weighted.address.clone(),
                        kind: settings.kind,
                        timeout: settings.timeout,
                    });
                }
            }
        }

        self.health.retain_known(&known);

        let max_concurrent = self.defaults.max_concurrent.max(1);
        let mut inflight = JoinSet::new();
        for target in targets {
            if inflight.len() >= max_concurrent {
                inflight.join_next().await;
            }
            let prober = Arc::clone(&self);
            inflight.spawn(async move { prober.probe(target).await });
        }
        while inflight.join_next().await.is_some() {}
    }

    async fn probe(&self, target: Target) {
        let ok = matches!(
            tokio::time::timeout(target.timeout, self.run_probe(&target)).await,
            Ok(true)
        );
        if ok {
            self.health.record_success(&target.address);
        } else {
            tracing::debug!(address = %target.address, kind = ?target.kind, "health probe failed");
            self.health.record_failure(&target.address);
        }
    }

    async fn run_probe(&self, target: &Target) -> bool {
        let connect_addr = format!("{}:{}", target.address.host, target.address.port);
        let Ok(mut stream) = TcpStream::connect(&connect_addr).await else {
            return false;
        };
        match target.kind {
            ProbeKind::Tcp => true,
            ProbeKind::StatusPing => crate::status::relay::status_exchange(
                &self.registry,
                &mut stream,
                &connect_addr,
                target.address.host.clone(),
                target.address.clone(),
                STATUS_PROTOCOL_VERSION,
            )
            .await
            .is_ok(),
        }
    }

    fn settings_for(&self, config: &ServerConfig) -> ActiveHealthConfig {
        config
            .active_health
            .clone()
            .unwrap_or_else(|| self.defaults.clone())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use crate::provider::ProviderId;

    fn prober(router: &Arc<DomainRouter>, health: &Arc<PassiveBackendHealth>) -> Arc<ActiveHealthProber> {
        Arc::new(ActiveHealthProber::new(
            Arc::clone(router),
            Arc::clone(health),
            Arc::new(infrarust_protocol::registry::build_default_registry()),
            &toml::from_str::<ProxyConfig>("").unwrap(),
        ))
    }

    fn config(address: &str) -> ServerConfig {
        toml::from_str(&format!(
            "name = \"probed\"\ndomains = [\"probed.test\"]\naddresses = [\"{address}\"]\n"
        ))
        .unwrap()
    }

    /// The regression test for the ejection that never recovers: a dead
    /// address must come back on its own once it answers again, and it must
    /// come back in slow start rather than at full weight.
    #[tokio::test]
    async fn sweep_reinstates_a_recovered_address_in_slow_start() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let router = Arc::new(DomainRouter::new());
        router.add(ProviderId::file("probed"), config(&format!("127.0.0.1:{port}")));
        let health = Arc::new(PassiveBackendHealth::with_threshold(1));
        let address: ServerAddress = format!("127.0.0.1:{port}").parse().unwrap();

        health.record_failure(&address);
        assert_ne!(health.snapshot(&address).state, BackendState::Healthy);

        // Still inside the ejection backoff: the sweep must not touch it.
        prober(&router, &health).sweep(false).await;
        assert_ne!(health.snapshot(&address).state, BackendState::Healthy);

        // Backoff elapsed but the port is still closed: the probe fails.
        health.rewind_failures_for_test(&address);
        prober(&router, &health).sweep(false).await;
        assert_ne!(health.snapshot(&address).state, BackendState::Healthy);

        // The server comes back.
        let _listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        health.rewind_failures_for_test(&address);
        prober(&router, &health).sweep(false).await;

        let snapshot = health.snapshot(&address);
        assert_eq!(snapshot.state, BackendState::Healthy);
        assert!(
            snapshot.healthy_since.is_some(),
            "a reinstated address must ramp back up"
        );
    }

    #[tokio::test]
    async fn sweep_leaves_healthy_addresses_alone_by_default() {
        let router = Arc::new(DomainRouter::new());
        router.add(ProviderId::file("probed"), config("127.0.0.1:1"));
        let health = Arc::new(PassiveBackendHealth::new());

        prober(&router, &health).sweep(true).await;

        let address: ServerAddress = "127.0.0.1:1".parse().unwrap();
        assert_eq!(health.snapshot(&address).state, BackendState::Healthy);
    }

    #[tokio::test]
    async fn sweep_forgets_addresses_that_left_the_config() {
        let router = Arc::new(DomainRouter::new());
        let health = Arc::new(PassiveBackendHealth::with_threshold(1));
        let gone: ServerAddress = "10.255.255.1:25565".parse().unwrap();
        health.record_failure(&gone);
        assert_ne!(health.snapshot(&gone).state, BackendState::Healthy);

        prober(&router, &health).sweep(false).await;
        assert_eq!(health.snapshot(&gone).state, BackendState::Healthy);
    }
}
