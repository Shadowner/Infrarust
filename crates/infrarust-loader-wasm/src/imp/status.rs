use std::sync::{Mutex, PoisonError};

use infrarust_api::plugin::{
    PluginFault, PluginHealth, PluginQueueStats, PluginRestarts, PluginRuntimeStatus, QueueWindow,
};
use infrarust_config::WasmRecoveryConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Healthy,
    Recovering(Option<tokio::time::Instant>),
    Quarantined(tokio::time::Instant),
    Stopped,
}

impl Phase {
    fn health(self, now: tokio::time::Instant) -> PluginHealth {
        match self {
            Self::Healthy => PluginHealth::Healthy,
            Self::Recovering(until) => PluginHealth::Recovering {
                retry_in: until.map(|until| until.saturating_duration_since(now)),
            },
            Self::Quarantined(until) => PluginHealth::Quarantined {
                retry_in: until.saturating_duration_since(now),
            },
            Self::Stopped => PluginHealth::Stopped,
        }
    }
}

struct LastFault {
    cause: String,
    at: tokio::time::Instant,
    generation: u64,
}

struct Reported {
    phase: Phase,
    generation: u64,
    restarts: Vec<tokio::time::Instant>,
    last_fault: Option<LastFault>,
}

pub(crate) struct StatusBoard {
    capacity: usize,
    recovery: WasmRecoveryConfig,
    reported: Mutex<Reported>,
}

impl StatusBoard {
    pub(crate) fn new(capacity: usize, recovery: WasmRecoveryConfig) -> Self {
        Self {
            capacity,
            recovery,
            reported: Mutex::new(Reported {
                phase: Phase::Healthy,
                generation: 1,
                restarts: Vec::new(),
                last_fault: None,
            }),
        }
    }

    fn reported(&self) -> std::sync::MutexGuard<'_, Reported> {
        self.reported.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn report(
        &self,
        phase: Phase,
        generation: u64,
        restarts: impl IntoIterator<Item = tokio::time::Instant>,
    ) {
        let mut reported = self.reported();
        reported.phase = phase;
        reported.generation = generation;
        reported.restarts.clear();
        reported.restarts.extend(restarts);
    }

    pub(crate) fn faulted(&self, cause: &str, generation: u64) {
        self.reported().last_fault = Some(LastFault {
            cause: cause.to_owned(),
            at: tokio::time::Instant::now(),
            generation,
        });
    }

    pub(crate) fn snapshot(&self, depth: usize) -> PluginRuntimeStatus {
        let queue = PluginQueueStats::new(depth, self.capacity, QueueWindow::default());
        let now = tokio::time::Instant::now();
        let reported = self.reported();
        let window = self.recovery.window;
        let in_window = reported
            .restarts
            .iter()
            .filter(|at| now.saturating_duration_since(**at) < window)
            .count();
        let status =
            PluginRuntimeStatus::new(reported.phase.health(now), reported.generation, queue)
                .with_restarts(PluginRestarts::new(
                    u32::try_from(in_window).unwrap_or(u32::MAX),
                    self.recovery.max_restarts,
                    window,
                ));
        match &reported.last_fault {
            Some(fault) => status.with_last_fault(PluginFault::new(
                fault.cause.as_str(),
                now.saturating_duration_since(fault.at),
                fault.generation,
            )),
            None => status,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::time::Duration;

    use super::*;

    fn recovery() -> WasmRecoveryConfig {
        WasmRecoveryConfig {
            max_restarts: 5,
            window: Duration::from_secs(300),
            backoff_initial: Duration::from_secs(1),
            backoff_max: Duration::from_secs(300),
        }
    }

    #[test]
    fn the_board_reports_the_phase_with_the_time_left_before_the_next_attempt() {
        let board = StatusBoard::new(1024, recovery());
        let fresh = board.snapshot(0);
        assert_eq!(fresh.health, PluginHealth::Healthy);
        assert_eq!(fresh.generation, 1);
        assert_eq!(fresh.queue.capacity, 1024);
        assert_eq!(fresh.restarts.in_window, 0);
        assert_eq!(fresh.restarts.max, 5);
        assert_eq!(fresh.last_fault, None);

        let until = tokio::time::Instant::now() + Duration::from_secs(30);
        board.report(Phase::Quarantined(until), 4, []);
        let quarantined = board.snapshot(5);
        assert_eq!(quarantined.generation, 4);
        assert_eq!(quarantined.queue.depth, 5);
        let PluginHealth::Quarantined { retry_in } = quarantined.health else {
            panic!("{quarantined:?}");
        };
        assert!(retry_in <= Duration::from_secs(30) && retry_in > Duration::from_secs(25));

        board.report(Phase::Recovering(None), 5, []);
        assert_eq!(
            board.snapshot(0).health,
            PluginHealth::Recovering { retry_in: None }
        );
        board.report(Phase::Stopped, 5, []);
        assert_eq!(board.snapshot(0).health, PluginHealth::Stopped);
    }

    #[tokio::test(start_paused = true)]
    async fn restarts_leave_the_count_once_they_are_older_than_the_window() {
        let board = StatusBoard::new(16, recovery());
        let t0 = tokio::time::Instant::now();
        board.report(Phase::Healthy, 3, [t0, t0 + Duration::from_secs(100)]);
        board.faulted("the guest trapped: boom", 2);
        tokio::time::advance(Duration::from_secs(250)).await;

        let status = board.snapshot(0);
        assert_eq!(status.restarts.in_window, 2);
        assert_eq!(status.restarts.window, Duration::from_secs(300));
        let fault = status.last_fault.expect("a fault was noted");
        assert_eq!(fault.cause, "the guest trapped: boom");
        assert_eq!(fault.generation, 2);
        assert_eq!(fault.ago, Duration::from_secs(250));

        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(board.snapshot(0).restarts.in_window, 1);
    }
}
