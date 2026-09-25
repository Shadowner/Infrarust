use std::collections::VecDeque;
use std::time::Duration;

use infrarust_config::WasmRecoveryConfig;
use tokio::time::Instant;

use crate::consts::FAR_FUTURE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Restart,
    Quarantine { until: Instant, backoff: Duration },
}

pub(crate) struct RestartBudget {
    policy: WasmRecoveryConfig,
    restarts: VecDeque<Instant>,
    quarantines: u32,
}

impl RestartBudget {
    pub(crate) fn new(policy: WasmRecoveryConfig) -> Self {
        Self {
            policy,
            restarts: VecDeque::new(),
            quarantines: 0,
        }
    }

    pub(crate) fn policy(&self) -> &WasmRecoveryConfig {
        &self.policy
    }

    pub(crate) fn after_fault(&mut self, now: Instant) -> Verdict {
        self.forget_before(now);
        let allowed = usize::try_from(self.policy.max_restarts).unwrap_or(usize::MAX);
        if self.restarts.len() < allowed {
            self.quarantines = 0;
            self.restarts.push_back(now);
            return Verdict::Restart;
        }
        let backoff = self.backoff();
        self.quarantines = self.quarantines.saturating_add(1);
        Verdict::Quarantine {
            until: now.checked_add(backoff).unwrap_or_else(|| now + FAR_FUTURE),
            backoff,
        }
    }

    pub(crate) fn retry(&mut self, now: Instant) {
        self.forget_before(now);
        self.restarts.push_back(now);
    }

    pub(crate) fn restarts_in_window(&self) -> usize {
        self.restarts.len()
    }

    fn forget_before(&mut self, now: Instant) {
        while let Some(&oldest) = self.restarts.front()
            && now.saturating_duration_since(oldest) >= self.policy.window
        {
            self.restarts.pop_front();
        }
    }

    fn backoff(&self) -> Duration {
        let max = self.policy.backoff_max;
        2u32.checked_pow(self.quarantines)
            .and_then(|factor| self.policy.backoff_initial.checked_mul(factor))
            .map_or(max, |backoff| backoff.min(max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_restarts: u32, window: u64, initial: u64, max: u64) -> WasmRecoveryConfig {
        WasmRecoveryConfig {
            max_restarts,
            window: Duration::from_secs(window),
            backoff_initial: Duration::from_secs(initial),
            backoff_max: Duration::from_secs(max),
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn quarantine(at: Instant, backoff: u64) -> Verdict {
        Verdict::Quarantine {
            until: at + secs(backoff),
            backoff: secs(backoff),
        }
    }

    #[test]
    fn restarts_until_the_budget_is_spent_then_quarantines_with_a_doubling_backoff() {
        let mut budget = RestartBudget::new(policy(2, 300, 1, 5));
        let t0 = Instant::now();

        assert_eq!(budget.after_fault(t0), Verdict::Restart);
        assert_eq!(budget.after_fault(t0), Verdict::Restart);
        assert_eq!(budget.after_fault(t0), quarantine(t0, 1));

        let mut now = t0;
        for expected in [2, 4, 5, 5] {
            now += secs(1);
            budget.retry(now);
            assert_eq!(budget.after_fault(now), quarantine(now, expected));
        }
    }

    #[test]
    fn restarts_leaving_the_window_free_the_budget_and_reset_the_backoff() {
        let mut budget = RestartBudget::new(policy(1, 10, 1, 60));
        let t0 = Instant::now();

        assert_eq!(budget.after_fault(t0), Verdict::Restart);
        assert_eq!(
            budget.after_fault(t0 + secs(1)),
            quarantine(t0 + secs(1), 1)
        );
        budget.retry(t0 + secs(2));
        assert_eq!(
            budget.after_fault(t0 + secs(11)),
            quarantine(t0 + secs(11), 2),
            "the retry at t0+2s is still inside the 10s window"
        );
        assert_eq!(budget.after_fault(t0 + secs(13)), Verdict::Restart);
        assert_eq!(budget.restarts_in_window(), 1);
        assert_eq!(
            budget.after_fault(t0 + secs(14)),
            quarantine(t0 + secs(14), 1),
            "an immediate restart resets the backoff"
        );
    }

    #[test]
    fn no_restart_budget_quarantines_on_the_first_fault() {
        let mut budget = RestartBudget::new(policy(0, 300, 3, 300));
        let t0 = Instant::now();
        assert_eq!(budget.after_fault(t0), quarantine(t0, 3));
    }

    #[test]
    fn a_long_run_of_quarantines_stays_at_the_maximum_backoff() {
        let mut budget = RestartBudget::new(policy(0, 300, 1, 300));
        let t0 = Instant::now();
        for _ in 0..100 {
            budget.after_fault(t0);
        }
        assert_eq!(budget.after_fault(t0), quarantine(t0, 300));
    }
}
