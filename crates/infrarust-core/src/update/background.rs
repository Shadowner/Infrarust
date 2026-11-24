use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::service::UpdateService;

const DEFAULT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timings {
    pub(crate) first_check: (Duration, Duration),
    pub(crate) stat_every: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            first_check: (Duration::from_secs(60), Duration::from_secs(600)),
            stat_every: Duration::from_secs(60),
        }
    }
}

impl UpdateService {
    pub fn spawn_background(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        self.spawn_background_with(shutdown, Timings::default())
    }

    pub(crate) fn spawn_background_with(
        self: &Arc<Self>,
        shutdown: CancellationToken,
        timings: Timings,
    ) -> JoinHandle<()> {
        tokio::spawn(run(Arc::clone(self), shutdown, timings))
    }
}

async fn run(service: Arc<UpdateService>, shutdown: CancellationToken, timings: Timings) {
    let mut stat =
        tokio::time::interval_at(Instant::now() + timings.stat_every, timings.stat_every);
    stat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut next_check = service
        .config
        .check
        .then(|| Instant::now() + jitter(timings.first_check));
    let mut reported = false;
    loop {
        let due = async {
            match next_check {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = stat.tick() => {
                let changed = service.observe_binary();
                if changed && !reported {
                    tracing::warn!("the binary on disk changed; run `restart` to apply it");
                }
                reported = changed;
            }
            () = due => {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    result = service.check(None) => {
                        if let Err(e) = result {
                            tracing::warn!(error = %e, "update check failed");
                        }
                    }
                }
                next_check = Some(Instant::now() + period(service.config.interval));
            }
        }
    }
}

fn jitter((min, max): (Duration, Duration)) -> Duration {
    let millis = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
    let (low, high) = (millis(min), millis(max).max(millis(min)));
    Duration::from_millis(rand::random_range(low..=high))
}

fn period(interval: Duration) -> Duration {
    if interval.is_zero() {
        DEFAULT_INTERVAL
    } else {
        interval
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::ffi::OsString;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use infrarust_config::UpdateConfig;

    use crate::binary::{BuildInfo, ExeSnapshot, RunningBinary};

    use super::super::keys::TrustedKeys;
    use super::super::testing::{
        FakeBinary, HttpServer, Reply, SigningKey, TEST_TARGET, write_executable,
    };
    use super::*;

    const BUILD: BuildInfo = BuildInfo {
        version: "2.0.0-beta.5",
        target: TEST_TARGET,
        official: true,
        docker: false,
    };

    fn service(dir: &tempfile::TempDir, config: UpdateConfig) -> Arc<UpdateService> {
        let exe = dir.path().join("infrarust");
        write_executable(&exe, &FakeBinary::new("2.0.0-beta.5").script());
        let binary = RunningBinary::new(BUILD, ExeSnapshot::at(exe), Vec::<OsString>::new());
        let keys = TrustedKeys::from_strs([SigningKey::generate().public()]).unwrap();
        UpdateService::with_keys(config, binary, None, keys)
    }

    async fn eventually(mut condition: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "condition never met");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_replaced_binary_is_noticed_even_without_checks() {
        let dir = tempfile::tempdir().unwrap();
        let service = service(
            &dir,
            UpdateConfig {
                check: false,
                url: "http://127.0.0.1:9/releases".to_string(),
                ..UpdateConfig::default()
            },
        );
        let shutdown = CancellationToken::new();
        let task = service.spawn_background_with(
            shutdown.clone(),
            Timings {
                first_check: (Duration::ZERO, Duration::ZERO),
                stat_every: Duration::from_millis(20),
            },
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(!service.status().binary_changed);

        let staged = dir.path().join(".infrarust.new");
        std::fs::write(&staged, b"another build").unwrap();
        std::fs::rename(&staged, dir.path().join("infrarust")).unwrap();
        eventually(|| service.status().binary_changed).await;
        assert!(service.status().last_check.is_none());

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn checks_start_after_the_delay_and_repeat_every_interval() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let server = HttpServer::start(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Reply::ok(b"[]".to_vec())
        })
        .await;
        let service = service(
            &dir,
            UpdateConfig {
                check: true,
                interval: Duration::from_millis(50),
                url: server.url("/releases"),
                ..UpdateConfig::default()
            },
        );
        let shutdown = CancellationToken::new();
        let task = service.spawn_background_with(
            shutdown.clone(),
            Timings {
                first_check: (Duration::from_millis(30), Duration::from_millis(30)),
                stat_every: Duration::from_secs(60),
            },
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        eventually(|| calls.load(Ordering::SeqCst) >= 3).await;
        let status = service.status();
        assert!(status.last_check.is_some());
        assert!(status.last_error.is_none());
        assert!(status.available.is_none());

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        let after = calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(calls.load(Ordering::SeqCst), after);
    }

    #[tokio::test]
    async fn disabled_checks_never_reach_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let server = HttpServer::start(|_| Reply::ok(b"[]".to_vec())).await;
        let service = service(
            &dir,
            UpdateConfig {
                check: false,
                interval: Duration::from_millis(10),
                url: server.url("/releases"),
                ..UpdateConfig::default()
            },
        );
        let shutdown = CancellationToken::new();
        let task = service.spawn_background_with(
            shutdown.clone(),
            Timings {
                first_check: (Duration::ZERO, Duration::ZERO),
                stat_every: Duration::from_millis(10),
            },
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.cancel();
        task.await.unwrap();
        assert!(server.requests().is_empty());
        assert!(!service.status().checks_enabled);
    }

    #[test]
    fn the_first_check_is_drawn_within_its_window() {
        let window = Timings::default().first_check;
        assert_eq!(window, (Duration::from_secs(60), Duration::from_secs(600)));
        for _ in 0..200 {
            let delay = jitter(window);
            assert!(delay >= window.0 && delay <= window.1, "{delay:?}");
        }
        assert_eq!(period(Duration::ZERO), DEFAULT_INTERVAL);
    }
}
