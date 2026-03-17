#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::services::scheduler::Scheduler;
use infrarust_core::services::scheduler::SchedulerImpl;

#[tokio::test]
async fn test_delay_executes() {
    let scheduler = SchedulerImpl::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    scheduler.delay(Duration::from_millis(10), Box::new(move || {
        c.fetch_add(1, Ordering::Relaxed);
    }));

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn test_interval_executes_multiple_times() {
    let scheduler = SchedulerImpl::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    scheduler.interval(Duration::from_millis(20), Box::new(move || {
        c.fetch_add(1, Ordering::Relaxed);
    }));

    tokio::time::sleep(Duration::from_millis(90)).await;
    let count = counter.load(Ordering::Relaxed);
    // Should have fired 3-4 times (20ms interval, 90ms total, skip first tick)
    assert!(count >= 2, "expected >= 2, got {count}");
}

#[tokio::test]
async fn test_cancel_stops_task() {
    let scheduler = SchedulerImpl::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    let handle = scheduler.interval(Duration::from_millis(10), Box::new(move || {
        c.fetch_add(1, Ordering::Relaxed);
    }));

    tokio::time::sleep(Duration::from_millis(50)).await;
    scheduler.cancel(handle);

    let count_at_cancel = counter.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let count_after = counter.load(Ordering::Relaxed);

    // Count should not have increased after cancel
    assert_eq!(count_at_cancel, count_after);
}
