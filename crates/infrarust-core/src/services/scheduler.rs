//! [`Scheduler`] implementation using tokio tasks.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use tokio::task::AbortHandle;

use infrarust_api::services::scheduler::{Scheduler, TaskHandle};

/// Tokio-backed scheduler implementation.
///
/// Each delayed or repeating task is a spawned tokio task.
/// Cancellation uses [`AbortHandle`].
pub struct SchedulerImpl {
    tasks: DashMap<TaskHandle, AbortHandle>,
    next_id: AtomicU64,
}

impl SchedulerImpl {
    /// Creates a new scheduler.
    pub fn new() -> Self {
        Self {
            tasks: DashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    fn next_handle(&self) -> TaskHandle {
        TaskHandle::new(self.next_id.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for SchedulerImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl infrarust_api::services::scheduler::private::Sealed for SchedulerImpl {}

impl Scheduler for SchedulerImpl {
    fn delay(&self, duration: Duration, task: Box<dyn FnOnce() + Send>) -> TaskHandle {
        let handle = self.next_handle();
        let tasks = self.tasks.clone();

        let join_handle = tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            task();
            tasks.remove(&handle);
        });

        self.tasks.insert(handle, join_handle.abort_handle());
        handle
    }

    fn interval(&self, period: Duration, task: Box<dyn Fn() + Send + Sync>) -> TaskHandle {
        let handle = self.next_handle();

        let join_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            // Skip the first immediate tick
            interval.tick().await;
            loop {
                interval.tick().await;
                task();
            }
        });

        self.tasks.insert(handle, join_handle.abort_handle());
        handle
    }

    fn cancel(&self, handle: TaskHandle) {
        if let Some((_, abort_handle)) = self.tasks.remove(&handle) {
            abort_handle.abort();
        }
    }
}
