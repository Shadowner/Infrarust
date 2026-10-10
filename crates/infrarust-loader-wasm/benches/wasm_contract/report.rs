use std::fmt::Write as _;
use std::time::{Duration, Instant};

pub(crate) fn runs() -> usize {
    std::env::var("CONTRACT_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3)
        .max(1)
}

pub(crate) fn scaled(count: usize) -> usize {
    let scale: f64 = std::env::var("CONTRACT_SCALE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1.0);
    ((count as f64) * scale).max(1.0) as usize
}

pub(crate) fn banner(runs: usize) {
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let cpus = std::thread::available_parallelism().map_or(0, |n| n.get());
    println!("# wasm_contract\n");
    println!(
        "runs per measurement: {runs}; cpus: {cpus}; loadavg at start: {}\n",
        load.trim()
    );
}

pub(crate) fn runtime(workers: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .unwrap()
}

pub(crate) fn per_iter_ns(start: Instant, iterations: usize) -> f64 {
    start.elapsed().as_nanos() as f64 / iterations.max(1) as f64
}

pub(crate) fn us(ns: f64) -> f64 {
    ns / 1_000.0
}

pub(crate) struct Samples {
    ns: Vec<f64>,
}

impl Samples {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            ns: Vec::with_capacity(capacity),
        }
    }

    pub(crate) fn push(&mut self, elapsed: Duration) {
        self.ns.push(elapsed.as_nanos() as f64);
    }

    pub(crate) fn percentile(&self, p: f64) -> f64 {
        if self.ns.is_empty() {
            return f64::NAN;
        }
        let mut sorted = self.ns.clone();
        sorted.sort_by(f64::total_cmp);
        let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[rank.min(sorted.len() - 1)]
    }
}

pub(crate) fn median(values: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

pub(crate) struct Table {
    title: String,
    unit: &'static str,
    rows: Vec<(String, Vec<f64>)>,
}

impl Table {
    pub(crate) fn new(title: impl Into<String>, unit: &'static str) -> Self {
        Self {
            title: title.into(),
            unit,
            rows: Vec::new(),
        }
    }

    pub(crate) fn record(&mut self, row: &str, value: f64) {
        if let Some((_, values)) = self.rows.iter_mut().find(|(name, _)| name == row) {
            values.push(value);
        } else {
            self.rows.push((row.to_owned(), vec![value]));
        }
    }

    pub(crate) fn print(&self) {
        let runs = self.rows.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
        let mut out = String::new();
        let _ = writeln!(out, "### {} ({})\n", self.title, self.unit);
        let _ = write!(out, "| measurement |");
        for run in 1..=runs {
            let _ = write!(out, " run {run} |");
        }
        let _ = writeln!(out, " median | spread |");
        let _ = write!(out, "|---|");
        for _ in 0..runs {
            let _ = write!(out, "---:|");
        }
        let _ = writeln!(out, "---:|---:|");
        for (name, values) in &self.rows {
            let _ = write!(out, "| {name} |");
            for run in 0..runs {
                match values.get(run) {
                    Some(value) => {
                        let _ = write!(out, " {} |", fmt(*value));
                    }
                    None => {
                        let _ = write!(out, " |");
                    }
                }
            }
            let _ = writeln!(
                out,
                " **{}** | ±{:.0}% |",
                fmt(median(values)),
                spread_pct(values)
            );
        }
        println!("{out}");
    }
}

fn fmt(value: f64) -> String {
    if value.is_nan() {
        "n/a".to_owned()
    } else if value.abs() >= 100.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

fn spread_pct(values: &[f64]) -> f64 {
    let med = median(values);
    if values.len() < 2 || med == 0.0 || med.is_nan() {
        return 0.0;
    }
    let max_dev = values
        .iter()
        .map(|v| (v - med).abs())
        .fold(0.0_f64, f64::max);
    100.0 * max_dev / med.abs()
}
