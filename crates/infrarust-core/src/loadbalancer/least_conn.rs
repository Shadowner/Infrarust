use std::sync::atomic::{AtomicUsize, Ordering};

use smallvec::SmallVec;

use super::slow_start::{SlowStartConfig, effective_weight};
use super::{BackendCandidate, LoadBalancer, split_health};

pub struct LeastConnections {
    tie_breaker: AtomicUsize,
    slow_start: Option<SlowStartConfig>,
}

impl LeastConnections {
    pub const fn new(slow_start: Option<SlowStartConfig>) -> Self {
        Self {
            tie_breaker: AtomicUsize::new(0),
            slow_start,
        }
    }
}

impl LoadBalancer for LeastConnections {
    fn name(&self) -> &'static str {
        "least_conn"
    }

    fn order<'a>(&self, candidates: &'a [BackendCandidate]) -> SmallVec<[&'a BackendCandidate; 4]> {
        let (healthy, unhealthy) = split_health(candidates);
        if healthy.is_empty() {
            return candidates.iter().collect();
        }

        let scores: SmallVec<[f64; 4]> = healthy
            .iter()
            .map(|c| {
                (c.active_connections + 1) as f64 / effective_weight(c, self.slow_start.as_ref())
            })
            .collect();

        let min = scores.iter().copied().fold(f64::INFINITY, f64::min);
        let tied: SmallVec<[usize; 4]> = scores
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == min)
            .map(|(i, _)| i)
            .collect();

        let pick = tied[self.tie_breaker.fetch_add(1, Ordering::Relaxed) % tied.len()];

        let mut rest: SmallVec<[usize; 4]> = (0..healthy.len()).filter(|i| *i != pick).collect();
        rest.sort_by(|a, b| {
            scores[*a]
                .partial_cmp(&scores[*b])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut result = SmallVec::new();
        result.push(healthy[pick]);
        result.extend(rest.into_iter().map(|i| healthy[i]));
        result.extend(unhealthy);
        result
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::loadbalancer::test_candidate;

    fn with_conns(host: &str, conns: usize) -> BackendCandidate {
        let mut c = test_candidate(host, true);
        c.active_connections = conns;
        c
    }

    fn pick<'a>(lb: &LeastConnections, candidates: &'a [BackendCandidate]) -> &'a str {
        lb.order(candidates)[0].address.host.as_str()
    }

    #[test]
    fn test_lc_picks_fewest_connections() {
        let candidates = vec![with_conns("a", 5), with_conns("b", 2), with_conns("c", 8)];
        assert_eq!(pick(&LeastConnections::new(None), &candidates), "b");
    }

    #[test]
    fn test_lc_tie_break_rotates() {
        let candidates = vec![with_conns("a", 1), with_conns("b", 1), with_conns("c", 9)];
        let lb = LeastConnections::new(None);
        let picks: Vec<&str> = (0..4).map(|_| pick(&lb, &candidates)).collect();
        assert_eq!(picks, ["a", "b", "a", "b"]);
    }

    #[test]
    fn test_lc_weighted_ratio() {
        // A: (4+1) / weight 2 = 2.5 ; B: (3+1) / weight 1 = 4.0 → A wins.
        let mut candidates = vec![with_conns("a", 4), with_conns("b", 3)];
        candidates[0].weight = 2;
        assert_eq!(pick(&LeastConnections::new(None), &candidates), "a");
    }

    #[test]
    fn test_lc_no_panic_with_moving_slow_start_scores() {
        // Regression: scores must be frozen within one `order()` call.
        // With a live ramp, recomputing scores between the min pass and
        // the tie pass left the tie set empty → `% 0` panic.
        let slow = SlowStartConfig {
            window: std::time::Duration::from_secs(60),
            aggression: 1.0,
        };
        let lb = LeastConnections::new(Some(slow));
        let mut candidates = vec![with_conns("a", 5), with_conns("b", 5)];
        let since = std::time::Instant::now() - std::time::Duration::from_secs(1);
        candidates[0].healthy_since = Some(since);
        candidates[1].healthy_since = Some(since);
        for _ in 0..1000 {
            assert_eq!(lb.order(&candidates).len(), 2);
        }
    }

    #[test]
    fn test_lc_rest_sorted_by_load() {
        let candidates = vec![
            with_conns("a", 5),
            with_conns("b", 1),
            with_conns("c", 3),
            with_conns("d", 9),
        ];
        let ordered = LeastConnections::new(None).order(&candidates);
        let hosts: Vec<&str> = ordered.iter().map(|c| c.address.host.as_str()).collect();
        assert_eq!(hosts, ["b", "c", "a", "d"]);
    }

    #[test]
    fn test_lc_long_lived_self_balances() {
        // Simulation: 90 long-lived sessions arrive; none ends. Each pick
        // increments the chosen address's count, as the registry would.
        let mut conns = [0usize; 3];
        let lb = LeastConnections::new(None);
        for _ in 0..90 {
            let candidates = vec![
                with_conns("0", conns[0]),
                with_conns("1", conns[1]),
                with_conns("2", conns[2]),
            ];
            let winner: usize = pick(&lb, &candidates).parse().unwrap();
            conns[winner] += 1;
        }
        assert_eq!(conns, [30, 30, 30], "long-lived sessions must balance");
    }

    #[test]
    fn test_lc_unhealthy_in_queue() {
        let mut candidates = vec![with_conns("a", 5), with_conns("free", 0)];
        candidates[1].state = crate::loadbalancer::BackendState::Unhealthy;
        let ordered = LeastConnections::new(None).order(&candidates);
        // Even with 0 connections, the unhealthy address stays last.
        assert_eq!(ordered[0].address.host, "a");
        assert_eq!(ordered[1].address.host, "free");
    }
}
