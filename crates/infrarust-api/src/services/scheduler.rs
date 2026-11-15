//! Task scheduler service.

use std::time::Duration;

use crate::BoxFuture;

#[doc(hidden)]
pub mod private {
    pub use crate::__sealed::Sealed;
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
pub trait Scheduler: Send + Sync + crate::__sealed::Sealed {
    fn delay(&self, duration: Duration, task: AsyncTask) -> TaskHandle;

    fn repeat(
        &self,
        period: Duration,
        initial_delay: Option<Duration>,
        task: RepeatingTask,
    ) -> TaskHandle;

    fn spawn(&self, task: BoxFuture<'static, ()>) -> TaskHandle;

    /// Cancelling or unloading the plugin stops the task only before it starts; a running task finishes.
    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send>) -> TaskHandle;

    /// Cancels a scheduled task.
    fn cancel(&self, handle: TaskHandle);
}

pub type AsyncTask = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;

pub type RepeatingTask = Box<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

pub trait SchedulerExt {
    fn run_after<F, Fut>(&self, duration: Duration, task: F) -> TaskHandle
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;

    fn run_every<F, Fut>(
        &self,
        period: Duration,
        initial_delay: Option<Duration>,
        task: F,
    ) -> TaskHandle
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static;

    fn run<Fut>(&self, task: Fut) -> TaskHandle
    where
        Fut: Future<Output = ()> + Send + 'static;

    fn run_blocking<F>(&self, task: F) -> TaskHandle
    where
        F: FnOnce() + Send + 'static;
}

impl<S: Scheduler + ?Sized> SchedulerExt for S {
    fn run_after<F, Fut>(&self, duration: Duration, task: F) -> TaskHandle
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.delay(duration, Box::new(move || Box::pin(task())))
    }

    fn run_every<F, Fut>(
        &self,
        period: Duration,
        initial_delay: Option<Duration>,
        task: F,
    ) -> TaskHandle
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.repeat(period, initial_delay, Box::new(move || Box::pin(task())))
    }

    fn run<Fut>(&self, task: Fut) -> TaskHandle
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.spawn(Box::pin(task))
    }

    fn run_blocking<F>(&self, task: F) -> TaskHandle
    where
        F: FnOnce() + Send + 'static,
    {
        self.spawn_blocking(Box::new(task))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct Inline {
        pending: Mutex<Vec<BoxFuture<'static, ()>>>,
        scheduled: Mutex<Vec<(Duration, Option<Duration>)>>,
    }

    impl crate::__sealed::Sealed for Inline {}

    impl Scheduler for Inline {
        fn delay(&self, duration: Duration, task: AsyncTask) -> TaskHandle {
            self.scheduled.lock().unwrap().push((duration, None));
            self.spawn(task())
        }

        fn repeat(
            &self,
            period: Duration,
            initial_delay: Option<Duration>,
            task: RepeatingTask,
        ) -> TaskHandle {
            self.scheduled.lock().unwrap().push((period, initial_delay));
            self.spawn(task());
            self.spawn(task())
        }

        fn spawn(&self, task: BoxFuture<'static, ()>) -> TaskHandle {
            let mut pending = self.pending.lock().unwrap();
            pending.push(task);
            TaskHandle::new(pending.len() as u64)
        }

        fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send>) -> TaskHandle {
            task();
            TaskHandle::new(0)
        }

        fn cancel(&self, _handle: TaskHandle) {}
    }

    #[tokio::test]
    async fn the_helpers_forward_closures_and_futures_to_the_erased_methods() {
        let inline = Arc::new(Inline::default());
        let scheduler: Arc<dyn Scheduler> = inline.clone();
        let runs = Arc::new(AtomicU32::new(0));
        let bump = {
            let runs = Arc::clone(&runs);
            move || {
                runs.fetch_add(1, Ordering::Relaxed);
            }
        };

        let once = bump.clone();
        scheduler.run_after(Duration::from_secs(1), move || async move { once() });
        let every = bump.clone();
        scheduler.run_every(
            Duration::from_secs(2),
            Some(Duration::from_secs(3)),
            move || {
                let every = every.clone();
                async move { every() }
            },
        );
        let now = bump.clone();
        scheduler.run(async move { now() });
        scheduler.run_blocking(bump);

        assert_eq!(
            *inline.scheduled.lock().unwrap(),
            [
                (Duration::from_secs(1), None),
                (Duration::from_secs(2), Some(Duration::from_secs(3)))
            ]
        );
        assert_eq!(runs.load(Ordering::Relaxed), 1);
        let pending = std::mem::take(&mut *inline.pending.lock().unwrap());
        for task in pending {
            task.await;
        }
        assert_eq!(runs.load(Ordering::Relaxed), 5);
    }
}
