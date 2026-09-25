//! Task scheduler service.

use std::time::Duration;

use crate::event::BoxFuture;

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
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

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

    /// Schedules a repeating task at a fixed interval, starting after an initial delay.
    fn interval_with_delay(
        &self,
        period: Duration,
        delay: Duration,
        task: Box<dyn Fn() + Send + Sync>,
    ) -> TaskHandle;

    fn spawn(&self, task: BoxFuture<'static, ()>) -> TaskHandle;

    fn delay_async(&self, duration: Duration, task: AsyncTask) -> TaskHandle;

    fn repeat(
        &self,
        period: Duration,
        initial_delay: Option<Duration>,
        task: RepeatingTask,
    ) -> TaskHandle;

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send>) -> TaskHandle;

    /// Cancels a scheduled task.
    fn cancel(&self, handle: TaskHandle);
}

pub type AsyncTask = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;

pub type RepeatingTask = Box<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;
