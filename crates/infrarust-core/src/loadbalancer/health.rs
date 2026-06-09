//! Backend health view consumed by the load balancer.

use std::time::Instant;

use dashmap::DashMap;

use infrarust_config::ServerAddress;
use infrarust_transport::ConnectAttemptObserver;

pub trait BackendHealthView: Send + Sync {
    /// `(healthy, healthy_since)`. `healthy_since` = since when the address
    /// is healthy (`None` if long stable / unknown).
    fn snapshot(&self, addr: &ServerAddress) -> (bool, Option<Instant>);
}

const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Copy)]
struct AddressHealth {
    consecutive_failures: u32,
    healthy: bool,
    healthy_since: Option<Instant>,
}

pub struct PassiveBackendHealth {
    state: DashMap<ServerAddress, AddressHealth>,
    failure_threshold: u32,
}

impl PassiveBackendHealth {
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_FAILURE_THRESHOLD)
    }

    pub fn with_threshold(failure_threshold: u32) -> Self {
        Self {
            state: DashMap::new(),
            failure_threshold: failure_threshold.max(1),
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

    pub fn mark_warming(&self, addr: &ServerAddress) {
        let mut entry = self.state.entry(addr.clone()).or_insert(AddressHealth {
            consecutive_failures: 0,
            healthy: false,
            healthy_since: None,
        });
        if !entry.healthy || entry.healthy_since.is_none() {
            entry.consecutive_failures = 0;
            entry.healthy = true;
            entry.healthy_since = Some(Instant::now());
        }
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
        health.mark_warming(&a);
        let (healthy, since) = health.snapshot(&a);
        assert!(healthy);
        assert!(since.is_some());
    }

    #[test]
    fn mark_warming_keeps_existing_ramp_start() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.mark_warming(&a);
        let first = health.snapshot(&a).1;
        std::thread::sleep(std::time::Duration::from_millis(5));
        health.mark_warming(&a);
        assert_eq!(health.snapshot(&a).1, first);
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
