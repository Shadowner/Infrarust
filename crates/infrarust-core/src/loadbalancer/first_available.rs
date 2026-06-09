use smallvec::SmallVec;

use super::{BackendCandidate, LoadBalancer, split_health};

pub struct FirstAvailable;

impl LoadBalancer for FirstAvailable {
    fn name(&self) -> &'static str {
        "first_available"
    }

    fn order<'a>(&self, candidates: &'a [BackendCandidate]) -> SmallVec<[&'a BackendCandidate; 4]> {
        let (mut healthy, unhealthy) = split_health(candidates);
        healthy.extend(unhealthy);
        healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loadbalancer::test_candidate;

    fn hosts<'a>(ordered: &[&'a BackendCandidate]) -> Vec<&'a str> {
        ordered.iter().map(|c| c.address.host.as_str()).collect()
    }

    #[test]
    fn test_first_available_config_order() {
        let candidates = vec![
            test_candidate("a", true),
            test_candidate("b", true),
            test_candidate("c", true),
        ];
        let lb = FirstAvailable;
        for _ in 0..3 {
            assert_eq!(hosts(&lb.order(&candidates)), ["a", "b", "c"]);
        }
    }

    #[test]
    fn test_first_available_skips_unhealthy_first() {
        let candidates = vec![
            test_candidate("a", false),
            test_candidate("b", true),
            test_candidate("c", true),
        ];
        assert_eq!(hosts(&FirstAvailable.order(&candidates)), ["b", "c", "a"]);
    }

    #[test]
    fn test_first_available_ignores_weight() {
        let mut candidates = vec![
            test_candidate("a", true),
            test_candidate("b", true),
            test_candidate("c", true),
        ];
        candidates[2].weight = 100;
        assert_eq!(hosts(&FirstAvailable.order(&candidates)), ["a", "b", "c"]);
    }
}
