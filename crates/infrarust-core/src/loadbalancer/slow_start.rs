//! Slow-start: progressive ramp-up of a freshly healthy backend's weight.
//!
//! A just-started Minecraft server is a JVM warming up (JIT, chunk loading,
//! cache population): flooding it from the first second causes timeouts.
//! Slow-start ramps its effective weight linearly from ~0 to nominal over a
//! configurable window. It is a **weight modulation**, not a strategy:
//! `RoundRobin` and `LeastConnections` both consume it through
//! [`effective_weight`] without dedicated code.

use std::time::{Duration, Instant};

use super::BackendCandidate;

pub(crate) const RAMP_FLOOR: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlowStartConfig {
    pub window: Duration,
    pub aggression: f64,
}

impl SlowStartConfig {
    pub(crate) fn ramp_factor(&self, healthy_since: Option<Instant>) -> f64 {
        match healthy_since {
            None => 1.0, // stable / unknown → full weight
            Some(since) => {
                let elapsed = since.elapsed().as_secs_f64();
                let window = self.window.as_secs_f64();
                if window <= 0.0 || elapsed >= window {
                    1.0
                } else {
                    (elapsed / window)
                        .powf(1.0 / self.aggression)
                        .clamp(RAMP_FLOOR, 1.0)
                }
            }
        }
    }
}

pub(crate) fn effective_weight(c: &BackendCandidate, slow_start: Option<&SlowStartConfig>) -> f64 {
    let base = f64::from(c.weight.max(1));
    match slow_start {
        Some(cfg) => (base * cfg.ramp_factor(c.healthy_since)).max(RAMP_FLOOR),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loadbalancer::{SelectionMode, test_candidate};

    fn cfg(window_secs: u64, aggression: f64) -> SlowStartConfig {
        SlowStartConfig {
            window: Duration::from_secs(window_secs),
            aggression,
        }
    }

    fn past(secs: u64) -> Instant {
        Instant::now() - Duration::from_secs(secs)
    }

    #[test]
    fn test_ramp_none_healthy_since_full_weight() {
        assert!((cfg(30, 1.0).ramp_factor(None) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ramp_zero_elapsed_floor() {
        let factor = cfg(30, 1.0).ramp_factor(Some(Instant::now()));
        assert!(factor >= RAMP_FLOOR);
        assert!(factor < 0.05, "fresh backend should be near the floor");
    }

    #[test]
    fn test_ramp_half_window_linear() {
        let factor = cfg(30, 1.0).ramp_factor(Some(past(15)));
        assert!((factor - 0.5).abs() < 0.01, "expected ~0.5, got {factor}");
    }

    #[test]
    fn test_ramp_after_window_full() {
        let factor = cfg(30, 1.0).ramp_factor(Some(past(31)));
        assert!((factor - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ramp_aggression_curve() {
        let linear = cfg(30, 1.0).ramp_factor(Some(past(15)));
        let soft = cfg(30, 2.0).ramp_factor(Some(past(15)));
        assert!((soft - 0.5_f64.sqrt()).abs() < 0.01, "got {soft}");
        assert!(soft > linear);
    }

    #[test]
    fn test_zero_window_full_weight() {
        let factor = cfg(0, 1.0).ramp_factor(Some(Instant::now()));
        assert!((factor - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_effective_weight_without_slow_start() {
        let mut c = test_candidate("a", true);
        c.weight = 3;
        c.healthy_since = Some(Instant::now()); // ignored without config
        assert!((effective_weight(&c, None) - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_effective_weight_zero_weight_clamped() {
        let mut c = test_candidate("a", true);
        c.weight = 0;
        assert!((effective_weight(&c, None) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_effective_weight_ramps() {
        let slow = cfg(30, 1.0);
        let mut c = test_candidate("a", true);
        c.weight = 4;
        c.healthy_since = Some(past(15));
        let eff = effective_weight(&c, Some(&slow));
        assert!((eff - 2.0).abs() < 0.1, "expected ~2.0, got {eff}");
    }

    #[test]
    fn test_slow_start_reduces_lc_selection() {
        use crate::loadbalancer::{LeastConnections, LoadBalancer};

        let lb = LeastConnections::new(Some(cfg(60, 1.0)));
        let mut warming = test_candidate("warming", true);
        warming.active_connections = 3;
        warming.healthy_since = Some(Instant::now());
        let mut stable = test_candidate("stable", true);
        stable.active_connections = 3;

        let candidates = vec![warming, stable];
        for _ in 0..5 {
            assert_eq!(
                lb.order(&candidates, SelectionMode::Advance)[0]
                    .address
                    .host,
                "stable"
            );
        }
    }

    #[test]
    fn test_slow_start_cold_backend_not_flooded() {
        use crate::loadbalancer::{LeastConnections, LoadBalancer};

        let lb = LeastConnections::new(Some(cfg(60, 1.0)));
        let mut warming = test_candidate("warming", true);
        warming.active_connections = 0;
        warming.healthy_since = Some(Instant::now());
        let mut stable = test_candidate("stable", true);
        stable.active_connections = 3;

        let candidates = vec![warming, stable];
        for _ in 0..5 {
            let ordered = lb.order(&candidates, SelectionMode::Advance);
            assert_eq!(ordered[0].address.host, "stable");
            // Still present as failover, never dropped.
            assert_eq!(ordered[1].address.host, "warming");
        }
    }

    #[test]
    fn test_slow_start_reduces_rr_selection() {
        use crate::loadbalancer::{LoadBalancer, RoundRobin};

        let lb = RoundRobin::new(Some(cfg(60, 1.0)));
        let mut warming = test_candidate("warming", true);
        warming.healthy_since = Some(Instant::now());
        let stable = test_candidate("stable", true);

        let candidates = vec![warming, stable];
        let warming_picks = (0..100)
            .filter(|_| {
                lb.order(&candidates, SelectionMode::Advance)[0]
                    .address
                    .host
                    == "warming"
            })
            .count();
        assert!(
            warming_picks <= 5,
            "warming backend picked {warming_picks}/100 times"
        );
    }
}
