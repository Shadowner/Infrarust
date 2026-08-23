//! Builds the `LoadBalancer` instance of a `ServerConfig`.

use std::sync::Arc;

use infrarust_config::{BalanceConfig, BalanceStrategy};

use super::{FirstAvailable, LeastConnections, LoadBalancer, RoundRobin, SlowStartConfig};

pub fn build_load_balancer(cfg: &BalanceConfig) -> Arc<dyn LoadBalancer> {
    let slow_start = cfg.slow_start.map(|window| SlowStartConfig {
        window,
        aggression: cfg.slow_start_aggression,
    });
    match cfg.strategy {
        BalanceStrategy::FirstAvailable => Arc::new(FirstAvailable),
        BalanceStrategy::RoundRobin => Arc::new(RoundRobin::new(slow_start)),
        BalanceStrategy::LeastConn => Arc::new(LeastConnections::new(slow_start)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_factory_first_available() {
        let lb = build_load_balancer(&BalanceConfig::default());
        assert_eq!(lb.name(), "first_available");
    }

    #[test]
    fn test_factory_round_robin_with_slow_start() {
        let lb = build_load_balancer(&BalanceConfig {
            strategy: BalanceStrategy::RoundRobin,
            slow_start: Some(Duration::from_secs(45)),
            slow_start_aggression: 2.0,
        });
        assert_eq!(lb.name(), "round_robin");
    }

    #[test]
    fn test_factory_least_conn() {
        let lb = build_load_balancer(&BalanceConfig {
            strategy: BalanceStrategy::LeastConn,
            slow_start: None,
            slow_start_aggression: 1.0,
        });
        assert_eq!(lb.name(), "least_conn");
    }
}
