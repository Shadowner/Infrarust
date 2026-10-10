use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use infrarust_config::WasmCodecQuarantineConfig;

const FIRST_PRUNE: usize = 1024;

#[derive(Debug)]
pub(crate) struct Quarantine {
    config: WasmCodecQuarantineConfig,
    keys: Mutex<Keys>,
}

#[derive(Debug)]
struct Keys {
    entries: HashMap<IpAddr, Arc<KeyEntry>>,
    next_prune: usize,
}

#[derive(Debug)]
pub(crate) struct KeyEntry {
    cuts: AtomicU64,
    state: Mutex<KeyState>,
}

#[derive(Debug)]
struct KeyState {
    faults: VecDeque<Instant>,
    open_until: Option<Instant>,
    next_backoff: Duration,
    reopened_at: Option<Instant>,
    faulted_after_reopen: bool,
}

#[derive(Debug)]
pub(crate) struct Ticket {
    entry: Arc<KeyEntry>,
    cut: u64,
}

impl Ticket {
    pub(crate) fn is_cut(&self) -> bool {
        self.entry.cuts.load(Ordering::Relaxed) != self.cut
    }
}

#[derive(Debug)]
pub(crate) enum Admission {
    Open(Ticket),
    Quarantined { retry_in: Duration },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Trip {
    pub(crate) faults: u32,
    pub(crate) window: Duration,
    pub(crate) backoff: Duration,
}

impl Quarantine {
    pub(crate) fn new(config: WasmCodecQuarantineConfig) -> Option<Self> {
        config.is_enabled().then(|| Self {
            config,
            keys: Mutex::new(Keys {
                entries: HashMap::new(),
                next_prune: FIRST_PRUNE,
            }),
        })
    }

    pub(crate) fn admit(&self, ip: IpAddr, now: Instant) -> Admission {
        let entry = {
            let mut keys = lock(&self.keys);
            if keys.entries.len() >= keys.next_prune {
                self.prune(&mut keys, now);
            }
            Arc::clone(keys.entries.entry(ip).or_insert_with(|| {
                Arc::new(KeyEntry {
                    cuts: AtomicU64::new(0),
                    state: Mutex::new(KeyState {
                        faults: VecDeque::new(),
                        open_until: None,
                        next_backoff: self.config.backoff_initial,
                        reopened_at: None,
                        faulted_after_reopen: false,
                    }),
                })
            }))
        };
        let state = lock(&entry.state);
        if let Some(until) = state.open_until.filter(|until| now < *until) {
            return Admission::Quarantined {
                retry_in: until - now,
            };
        }
        drop(state);
        let cut = entry.cuts.load(Ordering::Relaxed);
        Admission::Open(Ticket { entry, cut })
    }

    pub(crate) fn fault(&self, ticket: &Ticket, now: Instant) -> Option<Trip> {
        let entry = &ticket.entry;
        let mut state = lock(&entry.state);
        if state.open_until.is_some_and(|until| now < until) || ticket.is_cut() {
            return None;
        }
        let window = self.config.window;
        while state
            .faults
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= window)
        {
            state.faults.pop_front();
        }
        state.faults.push_back(now);
        if state
            .reopened_at
            .is_some_and(|reopened| now.saturating_duration_since(reopened) < window)
        {
            state.faulted_after_reopen = true;
        }
        if state.faults.len() < usize::try_from(self.config.faults).unwrap_or(usize::MAX) {
            return None;
        }
        let backoff = if state.reopened_at.is_some() && !state.faulted_after_reopen {
            self.config.backoff_initial
        } else {
            state.next_backoff
        };
        let until = now.checked_add(backoff).unwrap_or(now);
        state.open_until = Some(until);
        state.reopened_at = Some(until);
        state.faulted_after_reopen = false;
        state.next_backoff = backoff
            .saturating_mul(2)
            .min(self.config.backoff_max)
            .max(self.config.backoff_initial);
        state.faults.clear();
        entry.cuts.fetch_add(1, Ordering::Relaxed);
        Some(Trip {
            faults: self.config.faults,
            window,
            backoff,
        })
    }

    fn prune(&self, keys: &mut Keys, now: Instant) {
        let window = self.config.window;
        keys.entries.retain(|_, entry| {
            if Arc::strong_count(entry) > 1 {
                return true;
            }
            let state = lock(&entry.state);
            let open = state.open_until.is_some_and(|until| now < until);
            let recent_fault = state
                .faults
                .back()
                .is_some_and(|at| now.saturating_duration_since(*at) < window);
            let remembers_backoff = state
                .reopened_at
                .is_some_and(|reopened| now.saturating_duration_since(reopened) < window);
            open || recent_fault || remembers_backoff
        });
        keys.next_prune = FIRST_PRUNE.max(keys.entries.len().saturating_mul(2));
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        lock(&self.keys).entries.len()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn config(faults: u32) -> WasmCodecQuarantineConfig {
        WasmCodecQuarantineConfig {
            faults,
            window: 10 * SECOND,
            backoff_initial: 10 * SECOND,
            backoff_max: 40 * SECOND,
        }
    }

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([203, 0, 113, last])
    }

    fn ticket(quarantine: &Quarantine, ip: IpAddr, now: Instant) -> Ticket {
        match quarantine.admit(ip, now) {
            Admission::Open(ticket) => ticket,
            Admission::Quarantined { retry_in } => panic!("{ip} quarantined for {retry_in:?}"),
        }
    }

    fn fault_n(quarantine: &Quarantine, ip: IpAddr, now: Instant, n: u32) -> Option<Trip> {
        let mut last = None;
        for _ in 0..n {
            let ticket = ticket(quarantine, ip, now);
            last = quarantine.fault(&ticket, now);
        }
        last
    }

    #[test]
    fn zero_faults_turns_the_quarantine_off() {
        assert!(Quarantine::new(config(0)).is_none());
        assert!(Quarantine::new(config(1)).is_some());
    }

    #[test]
    fn the_nth_fault_in_a_window_quarantines_only_that_address() {
        let quarantine = Quarantine::new(config(5)).unwrap();
        let now = Instant::now();
        assert_eq!(fault_n(&quarantine, ip(1), now, 4), None);
        let trip = fault_n(&quarantine, ip(1), now, 1).unwrap();
        assert_eq!(trip.backoff, 10 * SECOND);
        assert_eq!(trip.faults, 5);

        assert!(matches!(
            quarantine.admit(ip(1), now + SECOND),
            Admission::Quarantined { retry_in } if retry_in == 9 * SECOND
        ));
        assert!(matches!(quarantine.admit(ip(2), now), Admission::Open(_)));
        assert!(matches!(
            quarantine.admit(ip(1), now + 10 * SECOND),
            Admission::Open(_)
        ));
    }

    #[test]
    fn faults_older_than_the_window_do_not_count() {
        let quarantine = Quarantine::new(config(3)).unwrap();
        let start = Instant::now();
        assert_eq!(fault_n(&quarantine, ip(1), start, 2), None);
        assert_eq!(fault_n(&quarantine, ip(1), start + 10 * SECOND, 2), None);
        assert!(fault_n(&quarantine, ip(1), start + 11 * SECOND, 1).is_some());
    }

    #[test]
    fn a_quarantine_cuts_the_live_instances_of_its_address_only() {
        let quarantine = Quarantine::new(config(2)).unwrap();
        let now = Instant::now();
        let live = ticket(&quarantine, ip(1), now);
        let other = ticket(&quarantine, ip(2), now);
        assert!(!live.is_cut());
        fault_n(&quarantine, ip(1), now, 2).unwrap();
        assert!(live.is_cut());
        assert!(!other.is_cut());
        assert_eq!(quarantine.fault(&live, now), None);
    }

    #[test]
    fn the_backoff_doubles_for_quarantines_in_a_row_up_to_the_max() {
        let quarantine = Quarantine::new(config(1)).unwrap();
        let mut now = Instant::now();
        let mut backoffs = Vec::new();
        for _ in 0..4 {
            let trip = fault_n(&quarantine, ip(1), now, 1).unwrap();
            backoffs.push(trip.backoff);
            now += trip.backoff + SECOND;
        }
        assert_eq!(
            backoffs,
            [10 * SECOND, 20 * SECOND, 40 * SECOND, 40 * SECOND]
        );
    }

    #[test]
    fn a_window_without_faults_after_a_quarantine_starts_the_backoff_over() {
        let quarantine = Quarantine::new(config(1)).unwrap();
        let start = Instant::now();
        let first = fault_n(&quarantine, ip(1), start, 1).unwrap();
        let second_at = start + first.backoff + SECOND;
        let second = fault_n(&quarantine, ip(1), second_at, 1).unwrap();
        assert_eq!(second.backoff, 20 * SECOND);
        let calm = second_at + second.backoff + 10 * SECOND;
        let third = fault_n(&quarantine, ip(1), calm, 1).unwrap();
        assert_eq!(third.backoff, 10 * SECOND);
    }

    #[test]
    fn idle_addresses_without_live_instances_are_forgotten() {
        let quarantine = Quarantine::new(config(5)).unwrap();
        let now = Instant::now();
        let held = ticket(&quarantine, ip(0), now);
        for n in 1..=u8::MAX {
            drop(ticket(&quarantine, ip(n), now));
        }
        for n in 0..4u8 {
            drop(ticket(&quarantine, IpAddr::from([198, 51, 100, n]), now));
        }
        let mut n = 0u32;
        while quarantine.tracked() < FIRST_PRUNE {
            n += 1;
            drop(ticket(&quarantine, IpAddr::from(n.to_be_bytes()), now));
        }
        drop(ticket(&quarantine, ip(0), now + 60 * SECOND));
        assert!(quarantine.tracked() < 8, "{}", quarantine.tracked());
        assert!(!held.is_cut());
    }
}
