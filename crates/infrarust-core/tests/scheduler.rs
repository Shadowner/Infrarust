#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::plugin::PluginContext;
use infrarust_api::services::scheduler::Scheduler;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::plugin::context_factory::{PluginContextFactory, PluginContextFactoryImpl};
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::tracking::TrackingScheduler;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use tokio::time::Instant;

mod mock_services;
use mock_services::{
    MockBanService, MockConfigService, MockLoadBalancerService, MockPlayerRegistry,
};

const MS: Duration = Duration::from_millis(1);

fn counter() -> (Arc<AtomicU32>, Arc<AtomicU32>) {
    let count = Arc::new(AtomicU32::new(0));
    (Arc::clone(&count), count)
}

async fn advance(by: Duration) {
    tokio::time::sleep(by).await;
}

#[tokio::test(start_paused = true)]
async fn a_delay_runs_once_after_its_duration() {
    let scheduler = SchedulerImpl::new();
    let (runs, seen) = counter();
    scheduler.delay(
        10 * MS,
        Box::new(move || {
            runs.fetch_add(1, Ordering::SeqCst);
        }),
    );

    advance(8 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    advance(5 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    advance(100 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert!(scheduler.is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_interval_runs_every_period_until_cancelled() {
    let scheduler = SchedulerImpl::new();
    let (runs, seen) = counter();
    let handle = scheduler.interval(
        10 * MS,
        Box::new(move || {
            runs.fetch_add(1, Ordering::SeqCst);
        }),
    );

    advance(35 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 3);
    scheduler.cancel(handle);
    advance(100 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 3);
    assert!(scheduler.is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_interval_with_delay_starts_after_the_delay() {
    let scheduler = SchedulerImpl::new();
    let (runs, seen) = counter();
    scheduler.interval_with_delay(
        10 * MS,
        3 * MS,
        Box::new(move || {
            runs.fetch_add(1, Ordering::SeqCst);
        }),
    );

    advance(6 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    advance(12 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn repeat_never_overlaps_and_waits_a_period_after_each_run() {
    let scheduler = SchedulerImpl::new();
    let runs: Arc<Mutex<Vec<(Instant, Instant)>>> = Arc::default();
    let active = Arc::new(AtomicU32::new(0));
    let (log, overlap) = (Arc::clone(&runs), Arc::clone(&active));
    let start = Instant::now();
    scheduler.repeat(
        10 * MS,
        None,
        Box::new(move || {
            let log = Arc::clone(&log);
            let overlap = Arc::clone(&overlap);
            Box::pin(async move {
                assert_eq!(overlap.fetch_add(1, Ordering::SeqCst), 0, "runs overlapped");
                let began = Instant::now();
                tokio::time::sleep(25 * MS).await;
                overlap.fetch_sub(1, Ordering::SeqCst);
                log.lock().unwrap().push((began, Instant::now()));
            })
        }),
    );

    advance(150 * MS).await;
    let runs = runs.lock().unwrap().clone();
    assert!(runs.len() >= 3, "{runs:?}");
    let first = runs[0].0 - start;
    assert!(first >= 10 * MS && first <= 11 * MS, "{first:?}");
    for pair in runs.windows(2) {
        let gap = pair[1].0 - pair[0].1;
        assert!(gap >= 10 * MS && gap <= 11 * MS, "{gap:?} in {runs:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn repeat_honours_an_initial_delay() {
    let scheduler = SchedulerImpl::new();
    let (runs, seen) = counter();
    scheduler.repeat(
        50 * MS,
        Some(Duration::ZERO),
        Box::new(move || {
            let runs = Arc::clone(&runs);
            Box::pin(async move {
                runs.fetch_add(1, Ordering::SeqCst);
            })
        }),
    );

    advance(5 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    advance(50 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn delay_async_awaits_the_future_after_the_delay() {
    let scheduler = SchedulerImpl::new();
    let (runs, seen) = counter();
    scheduler.delay_async(
        10 * MS,
        Box::new(move || {
            Box::pin(async move {
                tokio::time::sleep(5 * MS).await;
                runs.fetch_add(1, Ordering::SeqCst);
            })
        }),
    );

    advance(12 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(scheduler.len(), 1);
    advance(5 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert!(scheduler.is_empty());
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_spawned_future_drops_it_at_its_next_await() {
    struct Dropped(Arc<AtomicU32>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let scheduler = SchedulerImpl::new();
    let (drops, seen) = counter();
    let handle = scheduler.spawn(Box::pin(async move {
        let _guard = Dropped(drops);
        std::future::pending::<()>().await;
    }));
    advance(MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 0);

    scheduler.cancel(handle);
    advance(MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert!(scheduler.is_empty());
}

#[tokio::test(start_paused = true)]
async fn spawn_blocking_runs_off_the_async_workers() {
    let scheduler = SchedulerImpl::new();
    let (done, finished) = tokio::sync::oneshot::channel();
    scheduler.spawn_blocking(Box::new(move || {
        let _ = done.send(std::thread::current().name().map(str::to_string));
    }));
    let thread = finished.await.unwrap();
    assert_ne!(thread, std::thread::current().name().map(str::to_string));
}

#[tokio::test(start_paused = true)]
async fn a_repeating_task_survives_a_panicking_run() {
    let scheduler = SchedulerImpl::new();
    let (runs, seen) = counter();
    scheduler.interval(
        10 * MS,
        Box::new(move || {
            if runs.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("first run fails");
            }
        }),
    );
    advance(35 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn panicking_tasks_are_pruned_and_the_scheduler_keeps_going() {
    let scheduler = SchedulerImpl::new();
    scheduler.delay(MS, Box::new(|| panic!("sync boom")));
    scheduler.spawn(Box::pin(async { panic!("async boom") }));
    scheduler.delay_async(MS, Box::new(|| panic!("building the future fails")));
    let (runs, seen) = counter();
    scheduler.repeat(
        10 * MS,
        Some(Duration::ZERO),
        Box::new(move || {
            let n = runs.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                assert_ne!(n, 0, "the first run panics");
            })
        }),
    );

    advance(25 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 3);
    assert_eq!(scheduler.len(), 1, "only the repeating task is left");
}

#[tokio::test(start_paused = true)]
async fn tracking_scheduler_forgets_finished_tasks() {
    let scheduler = TrackingScheduler::new(Arc::new(SchedulerImpl::new()), "p");
    for _ in 0..3 {
        scheduler.delay(10 * MS, Box::new(|| {}));
    }
    assert_eq!(scheduler.tracked_count(), 3);
    advance(20 * MS).await;
    assert_eq!(scheduler.tracked_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_plugin_cannot_cancel_another_plugins_task() {
    let shared = Arc::new(SchedulerImpl::new());
    let owner = TrackingScheduler::new(Arc::clone(&shared), "owner");
    let other = TrackingScheduler::new(Arc::clone(&shared), "other");
    let (runs, seen) = counter();
    let handle = owner.delay(
        10 * MS,
        Box::new(move || {
            runs.fetch_add(1, Ordering::SeqCst);
        }),
    );

    other.cancel(handle);
    assert_eq!(other.cancel_all(), 0);
    advance(20 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

fn factory() -> PluginContextFactoryImpl {
    let services = PluginServices {
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        load_balancer_service: Arc::new(MockLoadBalancerService),
        plugin_registry: Arc::new(infrarust_core::plugin::PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(
            infrarust_core::filter::codec_registry::CodecFilterRegistryImpl::new(),
        ),
        transport_filter_registry: Arc::new(
            infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
        ),
        domain_router: Arc::new(infrarust_core::routing::DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        proxy_info: infrarust_api::services::proxy_info::ProxyInfo::default(),
        plugins_dir: std::path::PathBuf::from("plugins"),
    };
    PluginContextFactoryImpl::new(services, HashMap::new())
}

fn context(ctx: &Arc<dyn PluginContext>) -> &PluginContextImpl {
    ctx.as_any()
        .downcast_ref::<PluginContextImpl>()
        .expect("real PluginContextImpl")
}

#[tokio::test(start_paused = true)]
async fn disabling_a_plugin_cancels_every_task_it_scheduled() {
    let factory = factory();
    let ctx = factory.create_context("p");
    let bystander = factory.create_context("q");
    let (runs, seen) = counter();
    let (other_runs, other_seen) = counter();
    let handle = ctx.scheduler_handle();
    handle.repeat(
        10 * MS,
        None,
        Box::new(move || {
            let runs = Arc::clone(&runs);
            Box::pin(async move {
                runs.fetch_add(1, Ordering::SeqCst);
            })
        }),
    );
    ctx.scheduler().delay(time_to_never(), Box::new(|| {}));
    ctx.scheduler()
        .spawn(Box::pin(std::future::pending::<()>()));
    bystander.scheduler().interval(
        10 * MS,
        Box::new(move || {
            other_runs.fetch_add(1, Ordering::SeqCst);
        }),
    );

    advance(25 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 2);
    assert_eq!(context(&ctx).tracked_tasks(), 3);

    context(&ctx).cleanup();
    assert_eq!(context(&ctx).tracked_tasks(), 0);
    advance(50 * MS).await;
    assert_eq!(seen.load(Ordering::SeqCst), 2);
    assert_eq!(other_seen.load(Ordering::SeqCst), 7);
    assert_eq!(context(&bystander).tracked_tasks(), 1);
}

fn time_to_never() -> Duration {
    Duration::from_secs(3600)
}
