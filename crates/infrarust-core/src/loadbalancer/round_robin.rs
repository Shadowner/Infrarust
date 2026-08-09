use std::collections::HashMap;
use std::sync::Mutex;

use smallvec::SmallVec;

use infrarust_config::ServerAddress;

use super::slow_start::{SlowStartConfig, effective_weight};
use super::{BackendCandidate, LoadBalancer};

pub struct RoundRobin {
    state: Mutex<HashMap<ServerAddress, f64>>,
    slow_start: Option<SlowStartConfig>,
}

impl RoundRobin {
    pub fn new(slow_start: Option<SlowStartConfig>) -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            slow_start,
        }
    }
}

impl LoadBalancer for RoundRobin {
    fn name(&self) -> &'static str {
        "round_robin"
    }

    fn order_selectable<'a>(
        &self,
        selectable: &[&'a BackendCandidate],
    ) -> SmallVec<[&'a BackendCandidate; 4]> {
        let mut cw = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        cw.retain(|addr, _| selectable.iter().any(|c| &c.address == addr));

        let mut total = 0.0_f64;
        for c in selectable {
            let eff = effective_weight(c, self.slow_start.as_ref());
            *cw.entry(c.address.clone()).or_insert(0.0) += eff;
            total += eff;
        }

        let picked = selectable.iter().enumerate().max_by(|(_, a), (_, b)| {
            cw[&a.address]
                .partial_cmp(&cw[&b.address])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let Some((pick_idx, picked)) = picked else {
            return SmallVec::new();
        };

        if let Some(w) = cw.get_mut(&picked.address) {
            *w -= total;
        }
        drop(cw);

        let mut result = SmallVec::new();
        result.push(*picked);
        result.extend(
            selectable
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != pick_idx)
                .map(|(_, c)| *c),
        );
        result
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::loadbalancer::test_candidate;

    fn pick<'a>(lb: &RoundRobin, candidates: &'a [BackendCandidate]) -> &'a str {
        lb.order(candidates)[0].address.host.as_str()
    }

    #[test]
    fn test_rr_equal_weights_rotates() {
        let candidates = vec![
            test_candidate("a", true),
            test_candidate("b", true),
            test_candidate("c", true),
        ];
        let lb = RoundRobin::new(None);
        let picks: Vec<&str> = (0..6).map(|_| pick(&lb, &candidates)).collect();
        // Each address elected exactly once per cycle of 3.
        for cycle in picks.chunks(3) {
            let mut sorted = cycle.to_vec();
            sorted.sort_unstable();
            assert_eq!(sorted, ["a", "b", "c"], "uneven cycle: {picks:?}");
        }
        assert_eq!(picks[..3], picks[3..6]);
    }

    #[test]
    fn test_rr_single_address() {
        let candidates = vec![test_candidate("a", true)];
        let lb = RoundRobin::new(None);
        for _ in 0..5 {
            assert_eq!(pick(&lb, &candidates), "a");
        }
    }

    #[test]
    fn test_rr_weighted_distribution() {
        let mut candidates = vec![test_candidate("big", true), test_candidate("small", true)];
        candidates[0].weight = 3;
        let lb = RoundRobin::new(None);
        let big = (0..1000)
            .filter(|_| pick(&lb, &candidates) == "big")
            .count();
        assert_eq!(big, 750, "weight 3/1 must yield exactly 75% over 1000");
    }

    #[test]
    fn test_rr_smooth_no_bursting() {
        let mut candidates = vec![test_candidate("big", true), test_candidate("small", true)];
        candidates[0].weight = 3;
        let lb = RoundRobin::new(None);
        let picks: Vec<&str> = (0..12).map(|_| pick(&lb, &candidates)).collect();

        assert_ne!(
            picks[..4],
            ["big", "big", "big", "small"],
            "naive burst pattern: {picks:?}"
        );
        for window in picks.windows(4) {
            let smalls = window.iter().filter(|p| **p == "small").count();
            assert_eq!(smalls, 1, "uneven window in {picks:?}");
        }
    }

    #[test]
    fn test_rr_reconciles_added_address() {
        let mut candidates = vec![test_candidate("a", true), test_candidate("b", true)];
        let lb = RoundRobin::new(None);
        for _ in 0..4 {
            pick(&lb, &candidates);
        }
        candidates.push(test_candidate("c", true));
        let picks: Vec<&str> = (0..3).map(|_| pick(&lb, &candidates)).collect();
        assert!(picks.contains(&"c"), "new address never elected: {picks:?}");
    }

    #[test]
    fn test_rr_reconciles_removed_address() {
        let mut candidates = vec![
            test_candidate("a", true),
            test_candidate("b", true),
            test_candidate("c", true),
        ];
        let lb = RoundRobin::new(None);
        for _ in 0..3 {
            pick(&lb, &candidates);
        }
        candidates.pop();
        let picks: Vec<&str> = (0..4).map(|_| pick(&lb, &candidates)).collect();
        assert!(!picks.contains(&"c"));
        assert!(picks.contains(&"a") && picks.contains(&"b"));
    }

    #[test]
    fn test_rr_unhealthy_excluded_from_rotation() {
        let candidates = vec![
            test_candidate("a", true),
            test_candidate("sick", false),
            test_candidate("b", true),
        ];
        let lb = RoundRobin::new(None);
        for _ in 0..6 {
            let ordered = lb.order(&candidates);
            assert_ne!(ordered[0].address.host, "sick");
            assert_eq!(ordered.last().unwrap().address.host, "sick");
        }
    }

    #[test]
    fn test_rr_state_is_per_instance() {
        let candidates = vec![test_candidate("a", true), test_candidate("b", true)];
        let lb1 = RoundRobin::new(None);
        let lb2 = RoundRobin::new(None);
        let first1 = pick(&lb1, &candidates);
        pick(&lb1, &candidates);
        assert_eq!(pick(&lb2, &candidates), first1);
    }
}
