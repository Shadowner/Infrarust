use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::consts::{DEADLINE_MARGIN_DIVISOR, FAR_FUTURE, MAX_DEADLINE_MARGIN};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Deadline {
    expires: Instant,
    host_cutoff: Instant,
}

impl Deadline {
    #[cfg(test)]
    pub(crate) fn after(budget: Duration) -> Self {
        Self::within(budget, None)
    }

    pub(crate) fn within(budget: Duration, caller: Option<Instant>) -> Self {
        let now = Instant::now();
        let own = later(now, budget);
        let expires = caller.map_or(own, |caller| caller.clamp(now, own));
        let left = expires.saturating_duration_since(now);
        Self {
            expires,
            host_cutoff: later(now, left - margin(left)),
        }
    }

    pub(crate) const fn expires(&self) -> Instant {
        self.expires
    }

    pub(crate) fn has_passed(&self) -> bool {
        Instant::now() >= self.expires
    }
}

pub(crate) fn margin(budget: Duration) -> Duration {
    (budget / DEADLINE_MARGIN_DIVISOR).min(MAX_DEADLINE_MARGIN)
}

pub(crate) fn inside(budget: Duration) -> Duration {
    budget - margin(budget)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostCallLimit {
    until: Instant,
    set_by_deadline: bool,
}

impl HostCallLimit {
    pub(crate) fn new(timeout: Duration, deadline: Option<Deadline>) -> Self {
        let own = later(Instant::now(), timeout);
        match deadline {
            Some(deadline) if deadline.host_cutoff < own => Self {
                until: deadline.host_cutoff,
                set_by_deadline: true,
            },
            _ => Self {
                until: own,
                set_by_deadline: false,
            },
        }
    }

    pub(crate) async fn run<T>(self, call: impl Future<Output = T>) -> Result<T, HostCallTimeout> {
        tokio::time::timeout_at(self.until, call)
            .await
            .map_err(|_| self.expiry())
    }

    fn expiry(self) -> HostCallTimeout {
        if self.set_by_deadline {
            HostCallTimeout::NearDeadline
        } else {
            HostCallTimeout::Elapsed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum HostCallTimeout {
    #[error("host call timed out")]
    Elapsed,

    #[error("host call timed out: the plugin call is close to its deadline")]
    NearDeadline,
}

fn later(now: Instant, by: Duration) -> Instant {
    now.checked_add(by).unwrap_or_else(|| now + FAR_FUTURE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_margin_is_a_fifth_of_the_budget_capped_at_a_quarter_second() {
        assert_eq!(
            margin(Duration::from_millis(300)),
            Duration::from_millis(60)
        );
        assert_eq!(margin(Duration::from_secs(1)), Duration::from_millis(200));
        assert_eq!(margin(Duration::from_secs(10)), MAX_DEADLINE_MARGIN);
        assert_eq!(margin(Duration::ZERO), Duration::ZERO);
    }

    #[tokio::test]
    async fn a_host_call_stops_at_the_earlier_of_its_timeout_and_the_deadline_cutoff() {
        let deadline = Deadline::after(Duration::from_secs(10));

        let short = HostCallLimit::new(Duration::from_secs(1), Some(deadline));
        assert!(!short.set_by_deadline);
        let long = HostCallLimit::new(Duration::from_secs(30), Some(deadline));
        assert!(long.set_by_deadline);
        assert_eq!(long.until, deadline.host_cutoff);
        let unbounded = HostCallLimit::new(Duration::from_secs(30), None);
        assert!(!unbounded.set_by_deadline);
    }

    #[tokio::test]
    async fn a_passed_cutoff_fails_a_pending_call_at_once_but_keeps_a_ready_answer() {
        let spent = HostCallLimit::new(
            Duration::from_secs(30),
            Some(Deadline::after(Duration::ZERO)),
        );

        let pending = spent.run(std::future::pending::<()>()).await;
        assert_eq!(pending, Err(HostCallTimeout::NearDeadline));
        assert_eq!(spent.run(std::future::ready(7)).await, Ok(7));
    }

    #[test]
    fn a_zero_budget_has_passed_as_soon_as_it_is_made() {
        assert!(Deadline::after(Duration::ZERO).has_passed());
        assert!(!Deadline::after(Duration::from_secs(60)).has_passed());
    }

    #[test]
    fn a_caller_that_stops_waiting_sooner_brings_the_deadline_forward() {
        let now = Instant::now();
        let caller = now + Duration::from_millis(300);
        let tight = Deadline::within(Duration::from_secs(10), Some(caller));
        assert_eq!(tight.expires(), caller);
        assert!(tight.host_cutoff < caller);
        assert!(tight.host_cutoff >= caller - Duration::from_millis(60));

        let loose = Deadline::within(
            Duration::from_millis(100),
            Some(now + Duration::from_secs(10)),
        );
        assert!(loose.expires() < caller);

        let gone = Deadline::within(Duration::from_secs(10), Some(now - Duration::from_secs(1)));
        assert!(gone.has_passed());
    }

    #[test]
    fn an_event_keeps_a_margin_before_the_bus_gives_up() {
        assert_eq!(
            inside(Duration::from_millis(300)),
            Duration::from_millis(240)
        );
        assert_eq!(inside(Duration::from_secs(10)), Duration::from_millis(9750));
    }

    #[test]
    fn an_unrepresentable_budget_does_not_overflow() {
        let deadline = Deadline::after(Duration::MAX);
        assert!(!deadline.has_passed());
    }
}
