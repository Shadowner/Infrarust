use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::sync::lock;

#[derive(Debug)]
pub(crate) struct RateLimit {
    interval: Duration,
    burst: u32,
    window_ends: Option<Instant>,
    used: u32,
    suppressed: u64,
}

impl RateLimit {
    pub(crate) const fn new(interval: Duration, burst: u32) -> Self {
        Self {
            interval,
            burst,
            window_ends: None,
            used: 0,
            suppressed: 0,
        }
    }

    pub(crate) fn admit(&mut self) -> Option<u64> {
        self.admit_at(Instant::now())
    }

    fn admit_at(&mut self, now: Instant) -> Option<u64> {
        if self.window_ends.is_none_or(|ends| now >= ends) {
            self.window_ends = now.checked_add(self.interval);
            self.used = 0;
        }
        if self.used < self.burst {
            self.used += 1;
            Some(std::mem::take(&mut self.suppressed))
        } else {
            self.suppressed = self.suppressed.saturating_add(1);
            None
        }
    }
}

#[derive(Debug)]
pub(crate) struct SharedRateLimit(Mutex<RateLimit>);

impl SharedRateLimit {
    pub(crate) const fn new(interval: Duration, burst: u32) -> Self {
        Self(Mutex::new(RateLimit::new(interval, burst)))
    }

    pub(crate) fn admit(&self) -> Option<u64> {
        lock(&self.0).admit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn admits_a_burst_per_window_then_reports_what_it_suppressed() {
        let start = Instant::now();
        let mut limit = RateLimit::new(SECOND, 2);

        assert_eq!(limit.admit_at(start), Some(0));
        assert_eq!(limit.admit_at(start), Some(0));
        assert_eq!(limit.admit_at(start + SECOND / 2), None);
        assert_eq!(limit.admit_at(start + SECOND / 2), None);

        assert_eq!(limit.admit_at(start + SECOND), Some(2));
        assert_eq!(limit.admit_at(start + SECOND), Some(0));
        assert_eq!(limit.admit_at(start + SECOND), None);
    }

    #[test]
    fn a_single_admission_per_window_logs_once() {
        let start = Instant::now();
        let mut limit = RateLimit::new(Duration::from_secs(60), 1);

        assert_eq!(limit.admit_at(start), Some(0));
        for step in 1..=3 {
            assert_eq!(limit.admit_at(start + SECOND * step), None);
        }
        assert_eq!(limit.admit_at(start + Duration::from_secs(60)), Some(3));
    }

    #[test]
    fn a_shared_limit_reads_the_clock_itself() {
        let limit = SharedRateLimit::new(Duration::from_secs(60), 1);

        assert_eq!(limit.admit(), Some(0));
        assert_eq!(limit.admit(), None);
    }
}
