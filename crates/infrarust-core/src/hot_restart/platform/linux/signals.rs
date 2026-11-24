use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, Once, PoisonError};

pub use infrarust_transport::handover::HandoverSignal;
use infrarust_transport::handover::{
    SignalRecorder, block_signals, is_blocked, raise_signal, record_signal, unblock_signals,
};

static EARLY: AtomicU32 = AtomicU32::new(0);
static EARLY_TAKEN: AtomicBool = AtomicBool::new(false);
static EARLY_RECORDERS: Mutex<Vec<SignalRecorder>> = Mutex::new(Vec::new());
static LATE: AtomicU32 = AtomicU32::new(0);
static DEFERRED: AtomicU32 = AtomicU32::new(0);
static STOP_RECORDERS: Once = Once::new();

const STOPS: [HandoverSignal; 2] = [HandoverSignal::Terminate, HandoverSignal::Interrupt];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EarlySignals {
    pub terminate: bool,
    pub interrupt: bool,
    pub restart: bool,
    late: u32,
}

impl EarlySignals {
    const fn from_bits(bits: u32) -> Self {
        Self {
            terminate: bits & HandoverSignal::Terminate.bit() != 0,
            interrupt: bits & HandoverSignal::Interrupt.bit() != 0,
            restart: bits & HandoverSignal::Restart.bit() != 0,
            late: 0,
        }
    }

    pub const fn racing(self, signal: HandoverSignal) -> bool {
        self.late & signal.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        !(self.terminate || self.interrupt || self.restart)
    }

    pub const fn stop(self) -> Option<HandoverSignal> {
        if self.terminate {
            Some(HandoverSignal::Terminate)
        } else if self.interrupt {
            Some(HandoverSignal::Interrupt)
        } else {
            None
        }
    }
}

pub fn early_signals() -> EarlySignals {
    let recorders = std::mem::take(
        &mut *EARLY_RECORDERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    for recorder in recorders {
        recorder.stop();
    }
    EARLY_TAKEN.store(true, Ordering::SeqCst);
    EarlySignals::from_bits(EARLY.load(Ordering::SeqCst))
}

pub fn take_over_signals<T>(install: impl FnOnce() -> T) -> (T, EarlySignals) {
    let watching = !EARLY_RECORDERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty();
    let late: Vec<SignalRecorder> = if watching {
        HandoverSignal::ALL
            .into_iter()
            .filter_map(|signal| record_signal(signal, &LATE).ok())
            .collect()
    } else {
        Vec::new()
    };
    let installed = install();
    let mut early = early_signals();
    for recorder in late {
        recorder.stop();
    }
    early.late = LATE.load(Ordering::SeqCst);
    (installed, early)
}

pub fn defer_stop(signal: HandoverSignal) {
    DEFERRED.fetch_or(signal.bit(), Ordering::SeqCst);
}

pub(super) fn install_early() -> io::Result<()> {
    let mut recorders = EARLY_RECORDERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for signal in HandoverSignal::ALL {
        recorders.push(record_signal(signal, &EARLY)?);
    }
    unblock_signals(&HandoverSignal::ALL)
}

pub(super) fn record_stops() {
    STOP_RECORDERS.call_once(|| {
        for signal in STOPS {
            if let Err(error) = record_signal(signal, &DEFERRED) {
                eprintln!(
                    "infrarust: {} will not reach the next image: {error}",
                    signal.name()
                );
            }
        }
    });
}

pub(super) fn hold() -> io::Result<()> {
    block_signals(&HandoverSignal::ALL)
}

pub(super) fn release() {
    if let Err(error) = unblock_signals(&HandoverSignal::ALL) {
        eprintln!("infrarust: could not unblock the stop signals: {error}");
    }
}

pub(super) fn forward_pending_stop() {
    let mut bits = DEFERRED.load(Ordering::SeqCst);
    if !EARLY_TAKEN.load(Ordering::SeqCst) {
        bits |= EARLY.load(Ordering::SeqCst);
    }
    let Some(signal) = EarlySignals::from_bits(bits).stop() else {
        return;
    };
    if !is_blocked(signal).unwrap_or(false) {
        return;
    }
    if let Err(error) = raise_signal(signal) {
        eprintln!(
            "infrarust: {} could not be kept for the next image: {error}",
            signal.name()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_signals_read_their_bits() {
        assert!(EarlySignals::from_bits(0).is_empty());
        assert_eq!(
            EarlySignals::from_bits(HandoverSignal::Restart.bit()),
            EarlySignals {
                restart: true,
                ..EarlySignals::default()
            }
        );
        assert_eq!(
            EarlySignals::from_bits(0b111),
            EarlySignals {
                terminate: true,
                interrupt: true,
                restart: true,
                late: 0,
            }
        );
    }

    #[test]
    fn a_signal_seen_after_the_handlers_went_in_is_racing() {
        let early = EarlySignals {
            late: HandoverSignal::Terminate.bit(),
            ..EarlySignals::from_bits(0b011)
        };
        assert!(early.racing(HandoverSignal::Terminate));
        assert!(!early.racing(HandoverSignal::Interrupt));
        assert!(!EarlySignals::from_bits(0b111).racing(HandoverSignal::Restart));
    }

    #[test]
    fn a_terminate_wins_over_an_interrupt_and_a_restart_is_no_stop() {
        assert_eq!(
            EarlySignals::from_bits(0b011).stop(),
            Some(HandoverSignal::Terminate)
        );
        assert_eq!(
            EarlySignals::from_bits(0b110).stop(),
            Some(HandoverSignal::Interrupt)
        );
        assert_eq!(EarlySignals::from_bits(0b100).stop(), None);
    }
}
