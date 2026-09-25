use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use dashmap::DashMap;
use futures_util::FutureExt;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use tokio::time::{Instant, MissedTickBehavior};

use infrarust_api::event::BoxFuture;
use infrarust_api::services::scheduler::{AsyncTask, RepeatingTask, Scheduler, TaskHandle};

use crate::event_bus::CORE_OWNER;
use crate::event_bus::diagnostic::panic_message;

pub const MIN_PERIOD: Duration = Duration::from_millis(1);

type Tasks = DashMap<TaskHandle, Scheduled>;

struct Scheduled {
    owner: Arc<str>,
    abort: AbortHandle,
}

struct Finish {
    tasks: Weak<Tasks>,
    handle: TaskHandle,
}

impl Drop for Finish {
    fn drop(&mut self) {
        if let Some(tasks) = self.tasks.upgrade() {
            tasks.remove(&self.handle);
        }
    }
}

enum Body {
    Async(BoxFuture<'static, ()>),
    Blocking(Box<dyn FnOnce() + Send>),
}

pub struct SchedulerImpl {
    tasks: Arc<Tasks>,
    next_id: AtomicU64,
    core_owner: Arc<str>,
}

impl SchedulerImpl {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(DashMap::new()),
            next_id: AtomicU64::new(1),
            core_owner: Arc::from(CORE_OWNER),
        }
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn owned_count(&self, owner: &str) -> usize {
        self.tasks
            .iter()
            .filter(|entry| &*entry.value().owner == owner)
            .count()
    }

    pub fn cancel_owned(&self, owner: &str, handle: TaskHandle) -> bool {
        match self
            .tasks
            .remove_if(&handle, |_, scheduled| &*scheduled.owner == owner)
        {
            Some((_, scheduled)) => {
                scheduled.abort.abort();
                true
            }
            None => false,
        }
    }

    pub fn cancel_owner(&self, owner: &str) -> usize {
        let owned: Vec<TaskHandle> = self
            .tasks
            .iter()
            .filter(|entry| &*entry.value().owner == owner)
            .map(|entry| *entry.key())
            .collect();
        owned
            .into_iter()
            .filter(|handle| self.cancel_owned(owner, *handle))
            .count()
    }

    pub(crate) fn delay_for(
        &self,
        owner: &Arc<str>,
        duration: Duration,
        task: Box<dyn FnOnce() + Send>,
    ) -> TaskHandle {
        let label = Arc::clone(owner);
        self.launch(
            owner,
            Body::Async(Box::pin(async move {
                tokio::time::sleep(duration).await;
                run_sync(&label, task);
            })),
        )
    }

    pub(crate) fn interval_for(
        &self,
        owner: &Arc<str>,
        period: Duration,
        first: Duration,
        task: Box<dyn Fn() + Send + Sync>,
    ) -> TaskHandle {
        let label = Arc::clone(owner);
        let period = period.max(MIN_PERIOD);
        self.launch(
            owner,
            Body::Async(Box::pin(async move {
                let mut ticks = tokio::time::interval_at(Instant::now() + first, period);
                ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
                loop {
                    ticks.tick().await;
                    run_sync(&label, &task);
                }
            })),
        )
    }

    pub(crate) fn spawn_for(&self, owner: &Arc<str>, task: BoxFuture<'static, ()>) -> TaskHandle {
        let label = Arc::clone(owner);
        self.launch(
            owner,
            Body::Async(Box::pin(async move { run_async(&label, task).await })),
        )
    }

    pub(crate) fn delay_async_for(
        &self,
        owner: &Arc<str>,
        duration: Duration,
        task: AsyncTask,
    ) -> TaskHandle {
        let label = Arc::clone(owner);
        self.launch(
            owner,
            Body::Async(Box::pin(async move {
                tokio::time::sleep(duration).await;
                if let Some(future) = start(&label, task) {
                    run_async(&label, future).await;
                }
            })),
        )
    }

    pub(crate) fn repeat_for(
        &self,
        owner: &Arc<str>,
        period: Duration,
        initial_delay: Option<Duration>,
        task: RepeatingTask,
    ) -> TaskHandle {
        let label = Arc::clone(owner);
        let period = period.max(MIN_PERIOD);
        self.launch(
            owner,
            Body::Async(Box::pin(async move {
                tokio::time::sleep(initial_delay.unwrap_or(period)).await;
                loop {
                    if let Some(future) = start(&label, &task) {
                        run_async(&label, future).await;
                    }
                    tokio::time::sleep(period).await;
                }
            })),
        )
    }

    pub(crate) fn spawn_blocking_for(
        &self,
        owner: &Arc<str>,
        task: Box<dyn FnOnce() + Send>,
    ) -> TaskHandle {
        self.launch(owner, Body::Blocking(task))
    }

    fn launch(&self, owner: &Arc<str>, body: Body) -> TaskHandle {
        let handle = TaskHandle::new(self.next_id.fetch_add(1, Ordering::Relaxed));
        let finish = Finish {
            tasks: Arc::downgrade(&self.tasks),
            handle,
        };
        let (go, ready) = oneshot::channel::<()>();
        let abort = match body {
            Body::Async(future) => tokio::spawn(async move {
                let _finish = finish;
                if ready.await.is_ok() {
                    future.await;
                }
            })
            .abort_handle(),
            Body::Blocking(task) => {
                let label = Arc::clone(owner);
                tokio::task::spawn_blocking(move || {
                    let _finish = finish;
                    if ready.blocking_recv().is_ok() {
                        run_sync(&label, task);
                    }
                })
                .abort_handle()
            }
        };
        self.tasks.insert(
            handle,
            Scheduled {
                owner: Arc::clone(owner),
                abort,
            },
        );
        if go.send(()).is_err() {
            self.tasks.remove(&handle);
        }
        handle
    }
}

impl Default for SchedulerImpl {
    fn default() -> Self {
        Self::new()
    }
}

fn report(owner: &str, payload: &(dyn std::any::Any + Send)) {
    tracing::error!(
        plugin = %owner,
        panic = %panic_message(payload),
        "a scheduled task panicked; the scheduler keeps running"
    );
}

fn run_sync(owner: &str, task: impl FnOnce()) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(task)) {
        report(owner, payload.as_ref());
    }
}

fn start<F>(owner: &str, task: F) -> Option<BoxFuture<'static, ()>>
where
    F: FnOnce() -> BoxFuture<'static, ()>,
{
    match catch_unwind(AssertUnwindSafe(task)) {
        Ok(future) => Some(future),
        Err(payload) => {
            report(owner, payload.as_ref());
            None
        }
    }
}

async fn run_async(owner: &str, future: BoxFuture<'static, ()>) {
    if let Err(payload) = AssertUnwindSafe(future).catch_unwind().await {
        report(owner, payload.as_ref());
    }
}

impl infrarust_api::services::scheduler::private::Sealed for SchedulerImpl {}

impl Scheduler for SchedulerImpl {
    fn delay(&self, duration: Duration, task: Box<dyn FnOnce() + Send>) -> TaskHandle {
        self.delay_for(&self.core_owner, duration, task)
    }

    fn interval(&self, period: Duration, task: Box<dyn Fn() + Send + Sync>) -> TaskHandle {
        self.interval_for(&self.core_owner, period, period, task)
    }

    fn interval_with_delay(
        &self,
        period: Duration,
        delay: Duration,
        task: Box<dyn Fn() + Send + Sync>,
    ) -> TaskHandle {
        self.interval_for(&self.core_owner, period, delay, task)
    }

    fn spawn(&self, task: BoxFuture<'static, ()>) -> TaskHandle {
        self.spawn_for(&self.core_owner, task)
    }

    fn delay_async(&self, duration: Duration, task: AsyncTask) -> TaskHandle {
        self.delay_async_for(&self.core_owner, duration, task)
    }

    fn repeat(
        &self,
        period: Duration,
        initial_delay: Option<Duration>,
        task: RepeatingTask,
    ) -> TaskHandle {
        self.repeat_for(&self.core_owner, period, initial_delay, task)
    }

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send>) -> TaskHandle {
        self.spawn_blocking_for(&self.core_owner, task)
    }

    fn cancel(&self, handle: TaskHandle) {
        if let Some((_, scheduled)) = self.tasks.remove(&handle) {
            scheduled.abort.abort();
        }
    }
}
