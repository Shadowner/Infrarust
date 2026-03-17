//! Task scheduler service.

use std::time::Duration;

pub mod private {
    /// Sealed — only the proxy implements [`Scheduler`](super::Scheduler).
    pub trait Sealed {}
}

/// An opaque handle to a scheduled task.
///
/// Use with [`Scheduler::cancel`] to cancel a pending task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskHandle(u64);

impl TaskHandle {
    /// Creates a new task handle from a raw ID.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw handle ID.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Task scheduler for delayed and recurring work.
///
/// Obtained via [`PluginContext::scheduler()`](crate::plugin::PluginContext::scheduler).
///
/// Tasks run on the proxy's async runtime. The scheduler does not
/// depend on tokio in the API — the proxy provides the implementation.
pub trait Scheduler: Send + Sync + private::Sealed {
    /// Schedules a one-shot task after a delay.
    fn delay(&self, duration: Duration, task: Box<dyn FnOnce() + Send>) -> TaskHandle;

    /// Schedules a repeating task at a fixed interval.
    fn interval(&self, period: Duration, task: Box<dyn Fn() + Send + Sync>) -> TaskHandle;

    /// Cancels a scheduled task.
    fn cancel(&self, handle: TaskHandle);
}
