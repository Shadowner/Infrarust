//! Backend health view consumed by the load balancer.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;

use infrarust_config::ServerAddress;
use infrarust_transport::ConnectAttemptObserver;

pub trait BackendHealthView: Send + Sync {
    /// `(healthy, healthy_since)`. `healthy_since` = since when the address
    /// is healthy (`None` if long stable / unknown).
    fn snapshot(&self, addr: &ServerAddress) -> (bool, Option<Instant>);
}

const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

const PRUNE_INTERVAL_OPS: u32 = 1024;

const PRUNE_STABLE_AFTER: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy)]
struct AddressHealth {
    consecutive_failures: u32,
    healthy: bool,
    healthy_since: Option<Instant>,
}

pub struct PassiveBackendHealth {
    state: DashMap<ServerAddress, AddressHealth>,
    failure_threshold: u32,
    inserts_since_prune: AtomicU32,
}

impl PassiveBackendHealth {
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_FAILURE_THRESHOLD)
    }

    pub fn with_threshold(failure_threshold: u32) -> Self {
        Self {
            state: DashMap::new(),
            failure_threshold: failure_threshold.max(1),
            inserts_since_prune: AtomicU32::new(0),
        }
    }

    pub fn record_success(&self, addr: &ServerAddress) {
        let Some(mut entry) = self.state.get_mut(addr) else {
            return; // never failed → implicitly healthy and stable
        };
        entry.consecutive_failures = 0;
        if !entry.healthy {
            entry.healthy = true;
            entry.healthy_since = Some(Instant::now());
            tracing::info!(address = %addr, "backend address healthy again");
        }
    }

    pub fn record_failure(&self, addr: &ServerAddress) {
        {
            let mut entry = self.state.entry(addr.clone()).or_insert(AddressHealth {
                consecutive_failures: 0,
                healthy: true,
                healthy_since: None,
            });
            entry.consecutive_failures += 1;
            if entry.healthy && entry.consecutive_failures >= self.failure_threshold {
                entry.healthy = false;
                entry.healthy_since = None;
                tracing::warn!(
                    address = %addr,
                    failures = entry.consecutive_failures,
                    "backend address marked unhealthy"
                );
            }
        }
        self.maybe_prune();
    }

    pub fn mark_warming(&self, addr: &ServerAddress, ramp_window: Duration) {
        {
            let mut entry = self.state.entry(addr.clone()).or_insert(AddressHealth {
                consecutive_failures: 0,
                healthy: false,
                healthy_since: None,
            });
            entry.consecutive_failures = 0;
            let in_ramp = entry.healthy
                && entry
                    .healthy_since
                    .is_some_and(|since| since.elapsed() < ramp_window);
            if !in_ramp {
                entry.healthy = true;
                entry.healthy_since = Some(Instant::now());
            }
        }
        self.maybe_prune();
    }

    fn maybe_prune(&self) {
        if self.inserts_since_prune.fetch_add(1, Ordering::Relaxed) < PRUNE_INTERVAL_OPS {
            return;
        }
        self.inserts_since_prune.store(0, Ordering::Relaxed);
        self.state.retain(|_, e| {
            !e.healthy
                || e.consecutive_failures > 0
                || e.healthy_since
                    .is_some_and(|since| since.elapsed() < PRUNE_STABLE_AFTER)
        });
    }

    #[cfg(test)]
    fn tracked_addresses(&self) -> usize {
        self.state.len()
    }
}

impl Default for PassiveBackendHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendHealthView for PassiveBackendHealth {
    fn snapshot(&self, addr: &ServerAddress) -> (bool, Option<Instant>) {
        self.state
            .get(addr)
            .map_or((true, None), |e| (e.healthy, e.healthy_since))
    }
}

impl ConnectAttemptObserver for PassiveBackendHealth {
    fn on_attempt(&self, address: &ServerAddress, success: bool) {
        if success {
            self.record_success(address);
        } else {
            self.record_failure(address);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(host: &str) -> ServerAddress {
        ServerAddress {
            host: host.to_string(),
            port: 25565,
        }
    }

    #[test]
    fn unknown_address_is_healthy_and_stable() {
        let health = PassiveBackendHealth::new();
        assert_eq!(health.snapshot(&addr("a")), (true, None));
    }

    #[test]
    fn unhealthy_after_threshold_failures() {
        let health = PassiveBackendHealth::with_threshold(3);
        let a = addr("a");
        health.record_failure(&a);
        health.record_failure(&a);
        assert!(health.snapshot(&a).0, "below threshold → still healthy");
        health.record_failure(&a);
        assert!(!health.snapshot(&a).0);
    }

    #[test]
    fn success_resets_failure_streak() {
        let health = PassiveBackendHealth::with_threshold(3);
        let a = addr("a");
        health.record_failure(&a);
        health.record_failure(&a);
        health.record_success(&a);
        health.record_failure(&a);
        health.record_failure(&a);
        assert!(health.snapshot(&a).0, "streak must reset on success");
    }

    #[test]
    fn recovery_sets_healthy_since() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.record_failure(&a);
        assert_eq!(health.snapshot(&a), (false, None));
        health.record_success(&a);
        let (healthy, since) = health.snapshot(&a);
        assert!(healthy);
        assert!(since.is_some(), "recovery must reposition healthy_since");
    }

    #[test]
    fn success_on_stable_address_keeps_none_since() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.record_success(&a);
        assert_eq!(health.snapshot(&a), (true, None));
    }

    #[test]
    fn mark_warming_starts_ramp() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.mark_warming(&a, Duration::from_secs(45));
        let (healthy, since) = health.snapshot(&a);
        assert!(healthy);
        assert!(since.is_some());
    }

    #[test]
    fn mark_warming_within_window_keeps_ramp_start() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.mark_warming(&a, Duration::from_secs(45));
        let first = health.snapshot(&a).1;
        std::thread::sleep(Duration::from_millis(5));
        health.mark_warming(&a, Duration::from_secs(45));
        assert_eq!(
            health.snapshot(&a).1,
            first,
            "a startup wave must share one ramp origin"
        );
    }

    #[test]
    fn mark_warming_restarts_ramp_on_rewake() {
        let window = Duration::from_millis(10);
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        // First wake, then the ramp completes and the server sleeps.
        health.mark_warming(&a, window);
        let first = health.snapshot(&a).1.unwrap();
        std::thread::sleep(window * 2);
        // Second wake: the ramp must restart, not stay a no-op.
        health.mark_warming(&a, window);
        let second = health.snapshot(&a).1.unwrap();
        assert!(second > first, "re-wake must reposition healthy_since");
    }

    #[test]
    fn prune_sweeps_stable_entries_keeps_meaningful_ones() {
        let health = PassiveBackendHealth::with_threshold(3);
        let stable = addr("stable");
        // healthy, streak cleared, no ramp → equivalent to "no entry".
        health.record_failure(&stable);
        health.record_success(&stable);
        let warming = addr("warming");
        health.mark_warming(&warming, Duration::from_secs(45));
        assert_eq!(health.tracked_addresses(), 2);

        let flaky = addr("flaky");
        for _ in 0..=PRUNE_INTERVAL_OPS {
            health.record_failure(&flaky);
        }

        assert_eq!(health.tracked_addresses(), 2, "stable entry must be swept");
        assert!(!health.snapshot(&flaky).0);
        assert!(health.snapshot(&warming).1.is_some());
        assert_eq!(health.snapshot(&stable), (true, None));
    }

    #[test]
    fn observer_routes_attempts() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.on_attempt(&a, false);
        assert!(!health.snapshot(&a).0);
        health.on_attempt(&a, true);
        assert!(health.snapshot(&a).0);
    }
}
