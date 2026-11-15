use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use infrarust_api::plugin::QueueWindow;

const SLICE_MS: u64 = 10_000;
const SLICES: usize = 6;
const WINDOW: Duration = Duration::from_millis(SLICE_MS * SLICES as u64);
const EXACT: u64 = 4;
const STEPS_PER_OCTAVE: usize = 4;
const LAST_OCTAVE: u32 = 35;
const BUCKETS: usize = EXACT as usize + (LAST_OCTAVE as usize - 1) * STEPS_PER_OCTAVE;

struct Slice {
    epoch: AtomicU64,
    taken: AtomicU64,
    peak_depth: AtomicU64,
    max_micros: AtomicU64,
    counts: [AtomicU32; BUCKETS],
}

impl Slice {
    const fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            taken: AtomicU64::new(0),
            peak_depth: AtomicU64::new(0),
            max_micros: AtomicU64::new(0),
            counts: [const { AtomicU32::new(0) }; BUCKETS],
        }
    }

    fn start(&self, epoch: u64) {
        self.taken.store(0, Ordering::Relaxed);
        self.peak_depth.store(0, Ordering::Relaxed);
        self.max_micros.store(0, Ordering::Relaxed);
        for count in &self.counts {
            count.store(0, Ordering::Relaxed);
        }
        self.epoch.store(epoch, Ordering::Release);
    }
}

pub(crate) struct WaitWindow {
    origin: Instant,
    slices: [Slice; SLICES],
}

pub(crate) struct WaitRecorder {
    window: Arc<WaitWindow>,
}

impl WaitRecorder {
    pub(crate) fn record(&mut self, waited: Duration, depth: usize) {
        self.window.record_at(Instant::now(), waited, depth);
    }
}

impl WaitWindow {
    pub(crate) fn new() -> (WaitRecorder, Arc<Self>) {
        let window = Arc::new(Self {
            origin: Instant::now(),
            slices: [const { Slice::new() }; SLICES],
        });
        let recorder = WaitRecorder {
            window: Arc::clone(&window),
        };
        (recorder, window)
    }

    fn epoch(&self, now: Instant) -> u64 {
        let elapsed = u64::try_from(now.saturating_duration_since(self.origin).as_millis())
            .unwrap_or(u64::MAX);
        elapsed / SLICE_MS + 1
    }

    fn slice(&self, epoch: u64) -> &Slice {
        &self.slices[usize::try_from(epoch % SLICES as u64).unwrap_or(0)]
    }

    fn record_at(&self, now: Instant, waited: Duration, depth: usize) {
        let epoch = self.epoch(now);
        let slice = self.slice(epoch);
        if slice.epoch.load(Ordering::Relaxed) != epoch {
            slice.start(epoch);
        }
        let micros = u64::try_from(waited.as_micros()).unwrap_or(u64::MAX);
        bump(&slice.taken);
        bump_u32(&slice.counts[bucket(micros)]);
        raise(&slice.max_micros, micros);
        raise(&slice.peak_depth, u64::try_from(depth).unwrap_or(u64::MAX));
    }

    pub(crate) fn summary(&self) -> QueueWindow {
        self.summary_at(Instant::now())
    }

    fn summary_at(&self, now: Instant) -> QueueWindow {
        let current = self.epoch(now);
        let oldest = current.saturating_sub(SLICES as u64 - 1);
        let mut counts = [0u64; BUCKETS];
        let mut taken = 0u64;
        let mut peak_depth = 0u64;
        let mut max_micros = 0u64;
        for slice in &self.slices {
            let epoch = slice.epoch.load(Ordering::Acquire);
            if epoch < oldest || epoch > current {
                continue;
            }
            taken += slice.taken.load(Ordering::Relaxed);
            peak_depth = peak_depth.max(slice.peak_depth.load(Ordering::Relaxed));
            max_micros = max_micros.max(slice.max_micros.load(Ordering::Relaxed));
            for (total, count) in counts.iter_mut().zip(&slice.counts) {
                *total += u64::from(count.load(Ordering::Relaxed));
            }
        }
        let at = |quantile| Duration::from_micros(quantile_micros(&counts, quantile, max_micros));
        QueueWindow::new(
            WINDOW,
            taken,
            usize::try_from(peak_depth).unwrap_or(usize::MAX),
        )
        .waits(at(0.50), at(0.99), Duration::from_micros(max_micros))
    }
}

fn bump(counter: &AtomicU64) {
    counter.store(
        counter.load(Ordering::Relaxed).saturating_add(1),
        Ordering::Relaxed,
    );
}

fn bump_u32(counter: &AtomicU32) {
    counter.store(
        counter.load(Ordering::Relaxed).saturating_add(1),
        Ordering::Relaxed,
    );
}

fn raise(highest: &AtomicU64, value: u64) {
    if value > highest.load(Ordering::Relaxed) {
        highest.store(value, Ordering::Relaxed);
    }
}

fn bucket(micros: u64) -> usize {
    if micros < EXACT {
        return usize::try_from(micros).unwrap_or(0);
    }
    let octave = 63 - micros.leading_zeros();
    if octave > LAST_OCTAVE {
        return BUCKETS - 1;
    }
    let step = (micros >> (octave - 2)) & 3;
    EXACT as usize + (octave as usize - 2) * STEPS_PER_OCTAVE + usize::try_from(step).unwrap_or(0)
}

fn largest_in(bucket: usize) -> u64 {
    if bucket < EXACT as usize {
        return bucket as u64;
    }
    let octave = (bucket - EXACT as usize) / STEPS_PER_OCTAVE + 2;
    let step = ((bucket - EXACT as usize) % STEPS_PER_OCTAVE) as u64;
    let width = 1u64 << (octave - 2);
    (EXACT + step) * width + width - 1
}

fn quantile_micros(counts: &[u64; BUCKETS], quantile: f64, max_micros: u64) -> u64 {
    let taken: u64 = counts.iter().sum();
    if taken == 0 {
        return 0;
    }
    let rank = ((taken as f64 * quantile).ceil() as u64).clamp(1, taken);
    let mut seen = 0u64;
    for (bucket, count) in counts.iter().enumerate() {
        seen += count;
        if seen >= rank {
            return largest_in(bucket).min(max_micros);
        }
    }
    max_micros
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn every_bucket_holds_the_values_between_its_neighbours() {
        let mut previous_largest = None;
        for bucket_index in 0..BUCKETS {
            let largest = largest_in(bucket_index);
            assert_eq!(bucket(largest), bucket_index, "largest of {bucket_index}");
            if let Some(previous) = previous_largest {
                assert_eq!(
                    bucket(previous + 1),
                    bucket_index,
                    "smallest of {bucket_index}"
                );
            }
            previous_largest = Some(largest);
        }
        assert_eq!(bucket(u64::MAX), BUCKETS - 1);
    }

    #[test]
    fn a_bucket_is_at_most_a_quarter_octave_wide() {
        for micros in [5u64, 21, 999, 1_250, 20_000, 3_400_000] {
            let largest = largest_in(bucket(micros));
            assert!(largest >= micros, "{micros}");
            assert!(largest * 4 <= micros * 5 + 4, "{micros} -> {largest}");
        }
    }

    #[test]
    fn percentiles_come_from_the_waits_of_the_window() {
        let (_, window) = WaitWindow::new();
        let origin = window.origin;
        for _ in 0..98 {
            window.record_at(origin, Duration::from_micros(20), 1);
        }
        window.record_at(origin, ms(20), 7);
        window.record_at(origin + ms(9_000), ms(400), 3);

        let summary = window.summary_at(origin + ms(9_500));
        assert_eq!(summary.span, Duration::from_secs(60));
        assert_eq!(summary.taken, 100);
        assert_eq!(summary.peak_depth, 7);
        assert_eq!(summary.wait_max, ms(400));
        assert!(
            (Duration::from_micros(20)..=Duration::from_micros(23)).contains(&summary.wait_p50),
            "{summary:?}"
        );
        assert!((ms(20)..=ms(25)).contains(&summary.wait_p99), "{summary:?}");
    }

    #[test]
    fn the_largest_wait_caps_every_percentile() {
        let (_, window) = WaitWindow::new();
        let origin = window.origin;
        window.record_at(origin, Duration::from_micros(1_100), 1);
        let summary = window.summary_at(origin);
        assert_eq!(summary.wait_p50, Duration::from_micros(1_100));
        assert_eq!(summary.wait_p99, Duration::from_micros(1_100));
    }

    #[test]
    fn waits_older_than_the_window_are_forgotten() {
        let (_, window) = WaitWindow::new();
        let origin = window.origin;
        window.record_at(origin, ms(900), 40);
        window.record_at(origin + ms(30_000), ms(2), 2);

        let within = window.summary_at(origin + ms(55_000));
        assert_eq!(within.taken, 2);
        assert_eq!(within.wait_max, ms(900));

        let later = window.summary_at(origin + ms(65_000));
        assert_eq!(later.taken, 1);
        assert_eq!(later.peak_depth, 2);
        assert_eq!(later.wait_max, ms(2));

        window.record_at(origin + ms(125_000), ms(1), 1);
        let reused = window.summary_at(origin + ms(125_000));
        assert_eq!(reused.taken, 1, "a slice is emptied before it is reused");
        assert_eq!(reused.wait_max, ms(1));
    }

    #[test]
    fn an_idle_window_reports_no_waits() {
        let (_, window) = WaitWindow::new();
        let summary = window.summary_at(window.origin + ms(90_000));
        assert_eq!(summary.taken, 0);
        assert_eq!(summary.peak_depth, 0);
        assert_eq!(summary.wait_p50, Duration::ZERO);
        assert_eq!(summary.wait_p99, Duration::ZERO);
        assert_eq!(summary.wait_max, Duration::ZERO);
    }
}
