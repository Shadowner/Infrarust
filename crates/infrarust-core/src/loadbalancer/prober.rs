//! Active health probing.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use infrarust_config::{ActiveHealthConfig, ProbeKind, ProxyConfig, ServerAddress, ServerConfig};
use infrarust_protocol::registry::PacketRegistry;

use super::{BackendHealthView, BackendState, PassiveBackendHealth};
use crate::routing::DomainRouter;
use crate::status::STATUS_PROTOCOL_VERSION;

/// Floor on the sweep period, so a misconfigured `interval = 0` cannot turn
/// the prober into a busy loop.
const MIN_SWEEP_PERIOD: Duration = Duration::from_secs(1);

/// One task drives every server: each sweep only picks the addresses whose
/// own `[active_health]` cadence is due, and the next wake is the shortest
/// cadence in the routing table.
pub struct ActiveHealthProber {
    router: Arc<DomainRouter>,
    health: Arc<PassiveBackendHealth>,
    registry: Arc<PacketRegistry>,
    defaults: ActiveHealthConfig,
    last_probe: Mutex<HashMap<ServerAddress, Instant>>,
}

/// One address to probe, with the settings of the server it belongs to.
struct Target {
    address: ServerAddress,
    kind: ProbeKind,
    timeout: Duration,
}

/// What one server asks of the prober, once its own `[active_health]` block
/// has been resolved against the proxy-wide one.
///
/// Carries neither `enabled` (a server with probing off has no settings at
/// all) nor `max_concurrent` (a proxy-wide budget), so neither can be read
/// from the wrong block.
struct ProbeSettings {
    kind: ProbeKind,
    unhealthy_interval: Duration,
    probe_healthy: bool,
    interval: Duration,
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
            last_probe: Mutex::new(HashMap::new()),
        }
    }

    pub fn spawn(self: Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // The first sweep runs immediately, as the previous fixed ticker did.
            let mut period = Duration::ZERO;
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(period) => {}
                }
                period = Arc::clone(&self).sweep().await;
            }
        })
    }

    /// Probes everything that is due and returns how long to wait before the
    /// next sweep.
    async fn sweep(self: Arc<Self>) -> Duration {
        let (targets, period) = self.due_targets(Instant::now());

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

        period
    }

    /// Picks the addresses due for a probe and the shortest configured
    /// cadence, marking every picked address as probed at `now`.
    fn due_targets(&self, now: Instant) -> (Vec<Target>, Duration) {
        let mut known = HashSet::new();
        let mut targets = Vec::new();
        let mut period = self.defaults.unhealthy_interval;
        let mut last_probe = self.last_probe.lock().unwrap_or_else(|p| p.into_inner());

        for (_, config) in self.router.list_all() {
            let settings = self.settings_for(&config);
            if let Some(settings) = &settings {
                period = period.min(settings.unhealthy_interval);
                if settings.probe_healthy {
                    period = period.min(settings.interval);
                }
            }

            for weighted in &config.addresses {
                known.insert(weighted.address.clone());
                let Some(settings) = &settings else {
                    continue;
                };
                let healthy =
                    self.health.snapshot(&weighted.address).state == BackendState::Healthy;
                if healthy && !settings.probe_healthy {
                    continue;
                }
                let cadence = if healthy {
                    settings.interval
                } else {
                    settings.unhealthy_interval
                };
                let due = last_probe
                    .get(&weighted.address)
                    .is_none_or(|at| now.duration_since(*at) >= cadence);
                if !due {
                    continue;
                }
                if healthy || self.health.claim_probe(&weighted.address) {
                    last_probe.insert(weighted.address.clone(), now);
                    targets.push(Target {
                        address: weighted.address.clone(),
                        kind: settings.kind,
                        timeout: settings.timeout,
                    });
                }
            }
        }

        last_probe.retain(|addr, _| known.contains(addr));
        drop(last_probe);
        self.health.retain_known(&known);

        (targets, period.max(MIN_SWEEP_PERIOD))
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

    /// A server's own block replaces the proxy-wide one whole, `enabled`
    /// included: opting in is what a server with probing off proxy-wide does.
    fn settings_for(&self, config: &ServerConfig) -> Option<ProbeSettings> {
        let resolved = config.active_health.as_ref().unwrap_or(&self.defaults);
        resolved.enabled.then_some(ProbeSettings {
            kind: resolved.kind,
            unhealthy_interval: resolved.unhealthy_interval,
            probe_healthy: resolved.probe_healthy,
            interval: resolved.interval,
            timeout: resolved.timeout,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use crate::provider::ProviderId;

    fn prober(
        router: &Arc<DomainRouter>,
        health: &Arc<PassiveBackendHealth>,
    ) -> Arc<ActiveHealthProber> {
        prober_with(router, health, "")
    }

    fn prober_with(
        router: &Arc<DomainRouter>,
        health: &Arc<PassiveBackendHealth>,
        proxy_config: &str,
    ) -> Arc<ActiveHealthProber> {
        Arc::new(ActiveHealthProber::new(
            Arc::clone(router),
            Arc::clone(health),
            Arc::new(infrarust_protocol::registry::build_default_registry()),
            &toml::from_str::<ProxyConfig>(proxy_config).unwrap(),
        ))
    }

    fn config(address: &str) -> ServerConfig {
        toml::from_str(&format!(
            "name = \"probed\"\ndomains = [\"probed.test\"]\naddresses = [\"{address}\"]\n"
        ))
        .unwrap()
    }

    fn config_probing_every(name: &str, address: &str, interval: &str) -> ServerConfig {
        toml::from_str(&format!(
            "name = \"{name}\"\ndomains = [\"{name}.test\"]\naddresses = [\"{address}\"]\n\
             [active_health]\nprobe_healthy = true\ninterval = \"{interval}\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn per_server_cadence_gates_the_next_probe() {
        let router = Arc::new(DomainRouter::new());
        router.add(
            ProviderId::file("fast"),
            config_probing_every("fast", "127.0.0.1:1", "0s"),
        );
        router.add(
            ProviderId::file("slow"),
            config_probing_every("slow", "127.0.0.1:2", "10m"),
        );
        let health = Arc::new(PassiveBackendHealth::new());
        let prober = prober(&router, &health);

        let (first, period) = prober.due_targets(Instant::now());
        assert_eq!(first.len(), 2, "nothing probed yet, both are due");
        assert_eq!(
            period, MIN_SWEEP_PERIOD,
            "the shortest configured cadence drives the loop"
        );

        let (second, _) = prober.due_targets(Instant::now());
        let ports: Vec<u16> = second.iter().map(|t| t.address.port).collect();
        assert_eq!(ports, [1], "the slow server must not be probed again yet");
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
        router.add(
            ProviderId::file("probed"),
            config(&format!("127.0.0.1:{port}")),
        );
        let health = Arc::new(PassiveBackendHealth::with_threshold(1));
        let address: ServerAddress = format!("127.0.0.1:{port}").parse().unwrap();

        health.record_failure(&address);
        assert_ne!(health.snapshot(&address).state, BackendState::Healthy);

        // Still inside the ejection backoff: the sweep must not touch it.
        prober(&router, &health).sweep().await;
        assert_ne!(health.snapshot(&address).state, BackendState::Healthy);

        // Backoff elapsed but the port is still closed: the probe fails.
        health.rewind_failures_for_test(&address);
        prober(&router, &health).sweep().await;
        assert_ne!(health.snapshot(&address).state, BackendState::Healthy);

        // The server comes back.
        let _listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        health.rewind_failures_for_test(&address);
        prober(&router, &health).sweep().await;

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
        // One failed probe of the closed port is enough to eject.
        let health = Arc::new(PassiveBackendHealth::with_threshold(1));

        prober(&router, &health).sweep().await;

        let address: ServerAddress = "127.0.0.1:1".parse().unwrap();
        assert_eq!(health.snapshot(&address).state, BackendState::Healthy);
    }

    #[test]
    fn a_server_enabling_probing_is_probed_with_the_proxy_block_disabled() {
        let router = Arc::new(DomainRouter::new());
        router.add(
            ProviderId::file("opted_in"),
            config_probing_every("opted_in", "127.0.0.1:1", "0s"),
        );
        router.add(ProviderId::file("inheriting"), config("127.0.0.1:2"));
        let health = Arc::new(PassiveBackendHealth::new());

        let prober = prober_with(&router, &health, "[active_health]\nenabled = false\n");
        let (targets, _) = prober.due_targets(Instant::now());

        let ports: Vec<u16> = targets.iter().map(|t| t.address.port).collect();
        assert_eq!(ports, [1], "only the server that opted in must be probed");
    }

    #[test]
    fn a_server_disabling_probing_is_left_alone() {
        let router = Arc::new(DomainRouter::new());
        router.add(
            ProviderId::file("opted_out"),
            toml::from_str(
                "name = \"opted_out\"\ndomains = [\"opted_out.test\"]\n\
                 addresses = [\"127.0.0.1:1\"]\n[active_health]\nenabled = false\n",
            )
            .unwrap(),
        );
        let health = Arc::new(PassiveBackendHealth::with_threshold(1));
        let address: ServerAddress = "127.0.0.1:1".parse().unwrap();
        health.record_failure(&address);
        health.rewind_failures_for_test(&address);

        let prober = prober_with(&router, &health, "[active_health]\nprobe_healthy = true\n");
        let (targets, _) = prober.due_targets(Instant::now());

        assert!(targets.is_empty(), "the server opted out of probing");
    }

    #[tokio::test]
    async fn a_sweep_without_a_config_keeps_its_drain() {
        let router = Arc::new(DomainRouter::new());
        let health = Arc::new(PassiveBackendHealth::new());
        let address: ServerAddress = "10.255.255.1:25565".parse().unwrap();
        health.set_drained(&address, true);

        // A provider republishing a server: Removed, a sweep, then Added.
        prober(&router, &health).sweep().await;
        router.add(ProviderId::file("lobby"), config("10.255.255.1:25565"));

        assert_eq!(
            health.snapshot(&address).state,
            BackendState::Draining,
            "a drained backend must not silently rejoin the pool"
        );
    }

    #[tokio::test]
    async fn sweep_forgets_addresses_that_left_the_config() {
        let router = Arc::new(DomainRouter::new());
        let health = Arc::new(PassiveBackendHealth::with_threshold(1));
        let gone: ServerAddress = "10.255.255.1:25565".parse().unwrap();
        health.record_failure(&gone);
        assert_ne!(health.snapshot(&gone).state, BackendState::Healthy);

        prober(&router, &health).sweep().await;
        assert_eq!(health.snapshot(&gone).state, BackendState::Healthy);
    }
}
