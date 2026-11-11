#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use infrarust_server_manager::{
    PlayerCounter, ProviderStatus, ServerManagerError, ServerManagerService, ServerProvider,
    ServerState,
};
use tokio_util::sync::CancellationToken;

struct MockProvider {
    start_called: AtomicBool,
    stop_called: AtomicBool,
    polls: AtomicUsize,
    status: tokio::sync::Mutex<ProviderStatus>,
    status_after_start: ProviderStatus,
}

impl MockProvider {
    fn new(initial: ProviderStatus) -> Self {
        Self::with_status_after_start(initial, ProviderStatus::Starting)
    }

    fn with_status_after_start(initial: ProviderStatus, after_start: ProviderStatus) -> Self {
        Self {
            start_called: AtomicBool::new(false),
            stop_called: AtomicBool::new(false),
            polls: AtomicUsize::new(0),
            status: tokio::sync::Mutex::new(initial),
            status_after_start: after_start,
        }
    }

    async fn wait_for_polls(&self, count: usize) {
        while self.polls.load(Ordering::Acquire) < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl ServerProvider for MockProvider {
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            self.start_called.store(true, Ordering::Release);
            *self.status.lock().await = self.status_after_start;
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            self.stop_called.store(true, Ordering::Release);
            *self.status.lock().await = ProviderStatus::Stopped;
            Ok(())
        })
    }

    fn check_status(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            let status = *self.status.lock().await;
            self.polls.fetch_add(1, Ordering::AcqRel);
            Ok(status)
        })
    }

    fn provider_type(&self) -> &'static str {
        "mock"
    }
}

struct MockPlayerCounter;

impl PlayerCounter for MockPlayerCounter {
    fn count_by_server(&self, _server_id: &str) -> usize {
        0
    }
}

#[tokio::test]
async fn test_ensure_started_already_online() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(MockProvider::new(ProviderStatus::Running));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_secs(5),
    );

    // Set state to Online via initial health check
    service.initial_health_check().await;

    assert_eq!(service.get_state("test"), Some(ServerState::Online));

    // ensure_started should return immediately
    service.ensure_started("test").await.unwrap();
    // start should NOT have been called
    assert!(!provider.start_called.load(Ordering::Acquire));
}

#[tokio::test]
async fn test_ensure_started_sleeping_wakes_up() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(MockProvider::new(ProviderStatus::Stopped));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_secs(1),
    );

    // Start monitoring
    let shutdown = CancellationToken::new();
    let counter = Arc::new(MockPlayerCounter);
    let _handles = service.start_monitoring(counter, shutdown.clone());

    // Spawn ensure_started in background — it will block until Online
    let svc = Arc::clone(&service);
    let handle = tokio::spawn(async move { svc.ensure_started("test").await });

    // Give the ensure_started time to call start()
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(provider.start_called.load(Ordering::Acquire));

    // Simulate the provider becoming Running
    *provider.status.lock().await = ProviderStatus::Running;

    // Wait for ensure_started to complete (monitoring task should detect and notify)
    let result = tokio::time::timeout(Duration::from_secs(15), handle)
        .await
        .expect("should not timeout")
        .expect("task should not panic");

    assert!(result.is_ok());
    assert_eq!(service.get_state("test"), Some(ServerState::Online));

    shutdown.cancel();
}

#[tokio::test]
async fn test_ensure_started_timeout() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(MockProvider::new(ProviderStatus::Stopped));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(2), // Short timeout
        Duration::from_secs(1),
    );

    // Start monitoring
    let shutdown = CancellationToken::new();
    let counter = Arc::new(MockPlayerCounter);
    let _handles = service.start_monitoring(counter, shutdown.clone());

    // Provider stays in Starting forever (never becomes Running)
    let result = service.ensure_started("test").await;
    assert!(matches!(
        result,
        Err(ServerManagerError::StartTimeout { .. })
    ));

    shutdown.cancel();
}

#[tokio::test]
async fn test_ensure_started_multiple_waiters() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(MockProvider::new(ProviderStatus::Stopped));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_secs(1),
    );

    let shutdown = CancellationToken::new();
    let counter = Arc::new(MockPlayerCounter);
    let _handles = service.start_monitoring(counter, shutdown.clone());

    // Spawn 3 concurrent waiters
    let mut handles = vec![];
    for _ in 0..3 {
        let svc = Arc::clone(&service);
        handles.push(tokio::spawn(
            async move { svc.ensure_started("test").await },
        ));
    }

    // Give waiters time to register
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Simulate server becoming Running
    *provider.status.lock().await = ProviderStatus::Running;

    // All waiters should succeed
    for handle in handles {
        let result = tokio::time::timeout(Duration::from_secs(15), handle)
            .await
            .expect("should not timeout")
            .expect("task should not panic");
        assert!(result.is_ok());
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_auto_shutdown_after_idle() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(MockProvider::new(ProviderStatus::Running));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        Some(Duration::from_secs(1)), // Auto-shutdown after 1s idle
        Duration::from_secs(10),
        Duration::from_millis(500), // Fast polling for test
    );

    service.initial_health_check().await;
    assert_eq!(service.get_state("test"), Some(ServerState::Online));

    let shutdown = CancellationToken::new();
    let counter = Arc::new(MockPlayerCounter); // Always returns 0 players
    let _handles = service.start_monitoring(counter, shutdown.clone());

    // Wait long enough for auto-shutdown to trigger:
    // First poll at 3s (500ms * 6), sets last_player_seen
    // Second poll at 6s, elapsed >= 1s → stop
    tokio::time::sleep(Duration::from_secs(8)).await;

    assert!(provider.stop_called.load(Ordering::Acquire));

    shutdown.cancel();
}

#[tokio::test]
async fn test_register_custom_provider() {
    let service = ServerManagerService::new(&[], reqwest::Client::new());
    let provider = Arc::new(MockProvider::new(ProviderStatus::Running));

    service.register_server(
        "custom".to_string(),
        provider,
        None,
        Duration::from_secs(10),
        Duration::from_secs(5),
    );

    assert_eq!(service.get_state("custom"), Some(ServerState::Sleeping));

    service.initial_health_check().await;
    assert_eq!(service.get_state("custom"), Some(ServerState::Online));
}

#[tokio::test]
async fn test_get_all_managed() {
    let service = ServerManagerService::new(&[], reqwest::Client::new());

    service.register_server(
        "a".to_string(),
        Arc::new(MockProvider::new(ProviderStatus::Running)),
        None,
        Duration::from_secs(10),
        Duration::from_secs(5),
    );
    service.register_server(
        "b".to_string(),
        Arc::new(MockProvider::new(ProviderStatus::Stopped)),
        None,
        Duration::from_secs(10),
        Duration::from_secs(5),
    );

    let all = service.get_all_managed();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_server_not_found() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));

    let result = service.ensure_started("nonexistent").await;
    assert!(matches!(
        result,
        Err(ServerManagerError::ServerNotFound { .. })
    ));
}

#[tokio::test]
async fn test_manual_start_stop() {
    let service = ServerManagerService::new(&[], reqwest::Client::new());
    let provider = Arc::new(MockProvider::new(ProviderStatus::Stopped));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_secs(5),
    );

    service.start_server("test").await.unwrap();
    assert!(provider.start_called.load(Ordering::Acquire));
    assert_eq!(service.get_state("test"), Some(ServerState::Starting));

    service.stop_server("test").await.unwrap();
    assert!(provider.stop_called.load(Ordering::Acquire));
    assert_eq!(service.get_state("test"), Some(ServerState::Sleeping));
}

struct FailingStartProvider {
    release: tokio::sync::Notify,
}

impl ServerProvider for FailingStartProvider {
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            self.release.notified().await;
            Err(ServerManagerError::Provider {
                server_id: "test".to_string(),
                message: "docker daemon unreachable".to_string(),
            })
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn check_status(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send + '_>> {
        Box::pin(async { Ok(ProviderStatus::Stopped) })
    }

    fn provider_type(&self) -> &'static str {
        "failing"
    }
}

fn provider_message(result: &Result<(), ServerManagerError>) -> Option<&str> {
    match result {
        Err(ServerManagerError::Provider { message, .. }) => Some(message.as_str()),
        _ => None,
    }
}

#[tokio::test]
async fn test_joined_waiter_receives_the_provider_start_error() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(FailingStartProvider {
        release: tokio::sync::Notify::new(),
    });

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_secs(1),
    );

    let svc = Arc::clone(&service);
    let first = tokio::spawn(async move { svc.ensure_started("test").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(service.get_state("test"), Some(ServerState::Starting));

    let svc = Arc::clone(&service);
    let second = tokio::spawn(async move { svc.ensure_started("test").await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    provider.release.notify_one();

    let first = tokio::time::timeout(Duration::from_secs(5), first)
        .await
        .expect("should not timeout")
        .expect("task should not panic");
    let second = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("should not timeout")
        .expect("task should not panic");

    assert_eq!(provider_message(&first), Some("docker daemon unreachable"));
    assert_eq!(
        provider_message(&second),
        Some("docker daemon unreachable"),
        "joined waiter got {second:?}"
    );
    assert_eq!(service.get_state("test"), Some(ServerState::Crashed));
}

#[tokio::test]
async fn test_waiter_learns_the_server_went_back_to_sleep() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(MockProvider::new(ProviderStatus::Stopped));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_millis(100),
    );

    let shutdown = CancellationToken::new();
    let counter = Arc::new(MockPlayerCounter);
    let _handles = service.start_monitoring(counter, shutdown.clone());

    let svc = Arc::clone(&service);
    let handle = tokio::spawn(async move { svc.ensure_started("test").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(provider.start_called.load(Ordering::Acquire));
    provider.wait_for_polls(1).await;

    *provider.status.lock().await = ProviderStatus::Stopped;

    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("should not timeout")
        .expect("task should not panic");

    assert!(
        matches!(result, Err(ServerManagerError::WentToSleep { ref server_id }) if server_id == "test"),
        "got {result:?}"
    );
    assert_eq!(service.get_state("test"), Some(ServerState::Sleeping));

    shutdown.cancel();
}

#[tokio::test]
async fn test_stopped_polls_during_start_keep_the_waiter_waiting() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(MockProvider::with_status_after_start(
        ProviderStatus::Stopped,
        ProviderStatus::Stopped,
    ));

    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_millis(20),
    );

    let shutdown = CancellationToken::new();
    let counter = Arc::new(MockPlayerCounter);
    let _handles = service.start_monitoring(counter, shutdown.clone());

    let svc = Arc::clone(&service);
    let handle = tokio::spawn(async move { svc.ensure_started("test").await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(provider.start_called.load(Ordering::Acquire));

    let polls = provider.polls.load(Ordering::Acquire);
    provider.wait_for_polls(polls + 2).await;
    assert!(!handle.is_finished(), "waiter failed on a Stopped poll");
    assert_eq!(service.get_state("test"), Some(ServerState::Starting));

    *provider.status.lock().await = ProviderStatus::Running;

    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("should not timeout")
        .expect("task should not panic");
    assert!(result.is_ok(), "got {result:?}");
    assert_eq!(service.get_state("test"), Some(ServerState::Online));

    shutdown.cancel();
}

struct CountingProvider {
    starts: AtomicUsize,
    outcome: fn() -> Result<(), ServerManagerError>,
    status_after_start: ProviderStatus,
}

impl CountingProvider {
    fn refusing() -> Arc<Self> {
        Arc::new(Self {
            starts: AtomicUsize::new(0),
            outcome: || {
                Err(ServerManagerError::Provider {
                    server_id: "test".to_string(),
                    message: "refused".to_string(),
                })
            },
            status_after_start: ProviderStatus::Stopped,
        })
    }

    fn crashing() -> Arc<Self> {
        Arc::new(Self {
            starts: AtomicUsize::new(0),
            outcome: || Ok(()),
            status_after_start: ProviderStatus::Crashed,
        })
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::Acquire)
    }
}

impl ServerProvider for CountingProvider {
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::AcqRel);
            (self.outcome)()
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn check_status(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            Ok(if self.starts() == 0 {
                ProviderStatus::Stopped
            } else {
                self.status_after_start
            })
        })
    }

    fn provider_type(&self) -> &'static str {
        "counting"
    }
}

fn service_with(provider: Arc<dyn ServerProvider>) -> Arc<ServerManagerService> {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    service.register_server(
        "test".to_string(),
        provider,
        None,
        Duration::from_secs(10),
        Duration::from_millis(10),
    );
    service
}

fn is_throttled(result: &Result<(), ServerManagerError>) -> bool {
    matches!(result, Err(ServerManagerError::StartThrottled { .. }))
}

#[tokio::test]
async fn test_repeated_logins_after_a_failed_start_start_the_provider_once() {
    let provider = CountingProvider::refusing();
    let service = service_with(provider.clone());

    let first = service.ensure_started("test").await;
    assert_eq!(provider_message(&first), Some("refused"));
    assert_eq!(service.get_state("test"), Some(ServerState::Crashed));

    for _ in 0..5 {
        let result = service.ensure_started("test").await;
        assert!(is_throttled(&result), "got {result:?}");
    }

    assert_eq!(provider.starts(), 1);
    assert_eq!(service.get_state("test"), Some(ServerState::Crashed));
}

#[tokio::test]
async fn test_repeated_logins_after_a_crash_start_the_provider_once() {
    let provider = CountingProvider::crashing();
    let service = service_with(provider.clone());

    let shutdown = CancellationToken::new();
    let _handles = service.start_monitoring(Arc::new(MockPlayerCounter), shutdown.clone());

    let first = service.ensure_started("test").await;
    assert!(
        matches!(first, Err(ServerManagerError::ProcessExited { .. })),
        "got {first:?}"
    );

    for _ in 0..5 {
        let result = service.ensure_started("test").await;
        assert!(is_throttled(&result), "got {result:?}");
    }

    assert_eq!(provider.starts(), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn test_repeated_wakes_after_a_failed_start_start_the_provider_once() {
    let provider = CountingProvider::refusing();
    let service = service_with(provider.clone());

    let first = service.wake("test").await;
    assert_eq!(provider_message(&first), Some("refused"));
    assert_eq!(service.get_state("test"), Some(ServerState::Crashed));

    for _ in 0..5 {
        let result = service.wake("test").await;
        assert!(is_throttled(&result), "got {result:?}");
    }

    assert_eq!(provider.starts(), 1);
    assert_eq!(service.get_state("test"), Some(ServerState::Crashed));
}

#[tokio::test]
async fn test_logins_join_a_server_that_came_up_despite_a_failed_start() {
    let provider = Arc::new(CountingProvider {
        starts: AtomicUsize::new(0),
        outcome: || {
            Err(ServerManagerError::Provider {
                server_id: "test".to_string(),
                message: "panel timed out".to_string(),
            })
        },
        status_after_start: ProviderStatus::Running,
    });
    let service = service_with(provider.clone());

    let first = service.ensure_started("test").await;
    assert_eq!(provider_message(&first), Some("panel timed out"));
    assert_eq!(service.get_state("test"), Some(ServerState::Crashed));

    let shutdown = CancellationToken::new();
    let _handles = service.start_monitoring(Arc::new(MockPlayerCounter), shutdown.clone());

    tokio::time::timeout(Duration::from_secs(5), async {
        while service.get_state("test") != Some(ServerState::Online) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the monitor should notice the server is up");

    service.ensure_started("test").await.unwrap();
    assert_eq!(provider.starts(), 1);

    shutdown.cancel();
}

#[tokio::test]
async fn test_a_stopped_poll_keeps_a_failed_start_crashed() {
    let provider = CountingProvider::refusing();
    let service = service_with(provider.clone());

    let first = service.ensure_started("test").await;
    assert_eq!(provider_message(&first), Some("refused"));

    let shutdown = CancellationToken::new();
    let _handles = service.start_monitoring(Arc::new(MockPlayerCounter), shutdown.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(service.get_state("test"), Some(ServerState::Crashed));

    shutdown.cancel();
}

#[tokio::test]
async fn test_a_login_right_after_a_start_is_throttled() {
    let provider = CountingProvider::crashing();
    let service = service_with(provider.clone());

    service.start_server("test").await.unwrap();
    service.stop_server("test").await.unwrap();

    let result = service.ensure_started("test").await;
    assert!(is_throttled(&result), "got {result:?}");
    assert_eq!(provider.starts(), 1);
}

#[tokio::test]
async fn test_a_manual_start_bypasses_the_throttle() {
    let provider = CountingProvider::refusing();
    let service = service_with(provider.clone());

    let first = service.ensure_started("test").await;
    assert_eq!(provider_message(&first), Some("refused"));
    assert!(is_throttled(&service.ensure_started("test").await));

    let manual = service.start_server("test").await;
    assert_eq!(provider_message(&manual), Some("refused"));

    assert_eq!(provider.starts(), 2);
}

#[tokio::test]
async fn test_an_abandoned_login_does_not_abandon_the_start() {
    let service = Arc::new(ServerManagerService::new(&[], reqwest::Client::new()));
    let provider = Arc::new(FailingStartProvider {
        release: tokio::sync::Notify::new(),
    });
    service.register_server(
        "test".to_string(),
        provider.clone(),
        None,
        Duration::from_secs(10),
        Duration::from_secs(1),
    );

    let svc = Arc::clone(&service);
    let login = tokio::spawn(async move { svc.ensure_started("test").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(service.get_state("test"), Some(ServerState::Starting));

    login.abort();
    let _ = login.await;
    provider.release.notify_one();

    tokio::time::timeout(Duration::from_secs(5), async {
        while service.get_state("test") != Some(ServerState::Crashed) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the failed start should still be recorded");
}

struct LateFailingProvider {
    release: tokio::sync::Notify,
    status: ProviderStatus,
}

impl ServerProvider for LateFailingProvider {
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            self.release.notified().await;
            Err(ServerManagerError::Provider {
                server_id: "test".to_string(),
                message: "panel timed out".to_string(),
            })
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn check_status(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send + '_>> {
        Box::pin(async move { Ok(self.status) })
    }

    fn provider_type(&self) -> &'static str {
        "late-failing"
    }
}

fn service_recording_crashes(
    provider: Arc<LateFailingProvider>,
) -> (Arc<ServerManagerService>, Arc<AtomicUsize>) {
    let service = service_with(provider);
    let crashes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&crashes);
    service.add_on_state_change(Arc::new(move |_, _, new| {
        if new == ServerState::Crashed {
            counted.fetch_add(1, Ordering::AcqRel);
        }
    }));
    (service, crashes)
}

#[tokio::test]
async fn test_a_late_start_failure_does_not_crash_a_server_already_online() {
    let provider = Arc::new(LateFailingProvider {
        release: tokio::sync::Notify::new(),
        status: ProviderStatus::Running,
    });
    let (service, crashes) = service_recording_crashes(provider.clone());
    let shutdown = CancellationToken::new();
    let _handles = service.start_monitoring(Arc::new(MockPlayerCounter), shutdown.clone());

    tokio::time::timeout(Duration::from_secs(5), service.ensure_started("test"))
        .await
        .expect("the poll should report the server online")
        .unwrap();

    provider.release.notify_one();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(crashes.load(Ordering::Acquire), 0);
    assert_eq!(service.get_state("test"), Some(ServerState::Online));

    shutdown.cancel();
}

#[tokio::test]
async fn test_a_late_start_failure_does_not_overwrite_a_manual_stop() {
    let provider = Arc::new(LateFailingProvider {
        release: tokio::sync::Notify::new(),
        status: ProviderStatus::Stopped,
    });
    let (service, crashes) = service_recording_crashes(provider.clone());

    let svc = Arc::clone(&service);
    let login = tokio::spawn(async move { svc.ensure_started("test").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(service.get_state("test"), Some(ServerState::Starting));

    service.stop_server("test").await.unwrap();
    provider.release.notify_one();
    tokio::time::sleep(Duration::from_millis(100)).await;
    login.abort();

    assert_eq!(crashes.load(Ordering::Acquire), 0);
    assert_eq!(service.get_state("test"), Some(ServerState::Sleeping));
}

const PANEL_LAG: Duration = Duration::from_millis(200);

#[derive(Default)]
struct LaggingStopProvider {
    running: AtomicBool,
    stopped_at: std::sync::Mutex<Option<std::time::Instant>>,
}

impl ServerProvider for LaggingStopProvider {
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            self.running.store(true, Ordering::Release);
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            *self.stopped_at.lock().unwrap() = Some(std::time::Instant::now());
            self.running.store(false, Ordering::Release);
            Ok(())
        })
    }

    fn check_status(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send + '_>> {
        Box::pin(async move {
            let still_reported = self
                .stopped_at
                .lock()
                .unwrap()
                .is_some_and(|stopped_at| stopped_at.elapsed() < PANEL_LAG);
            Ok(if self.running.load(Ordering::Acquire) || still_reported {
                ProviderStatus::Running
            } else {
                ProviderStatus::Stopped
            })
        })
    }

    fn provider_type(&self) -> &'static str {
        "lagging"
    }
}

async fn wait_for_state(service: &ServerManagerService, state: ServerState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.get_state("test") != Some(state) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the server never became {state}"));
}

#[tokio::test]
async fn test_a_requested_stop_the_panel_reports_late_is_not_an_early_exit() {
    let service = service_with(Arc::new(LaggingStopProvider::default()));
    let shutdown = CancellationToken::new();
    let _handles = service.start_monitoring(Arc::new(MockPlayerCounter), shutdown.clone());

    for _ in 0..2 {
        service.start_server("test").await.unwrap();
        wait_for_state(&service, ServerState::Online).await;
        service.stop_server("test").await.unwrap();
        wait_for_state(&service, ServerState::Online).await;
        wait_for_state(&service, ServerState::Sleeping).await;
    }

    let result = service.ensure_started("test").await;
    let Err(ServerManagerError::StartThrottled { retry_after, .. }) = result else {
        panic!("a login right after a start is throttled, got {result:?}");
    };
    assert!(retry_after <= Duration::from_secs(30), "{retry_after:?}");

    shutdown.cancel();
}

#[tokio::test]
async fn test_an_exit_after_a_confirmed_stop_is_counted() {
    let provider = Arc::new(MockProvider::with_status_after_start(
        ProviderStatus::Stopped,
        ProviderStatus::Running,
    ));
    let service = service_with(provider.clone());
    let shutdown = CancellationToken::new();
    let _handles = service.start_monitoring(Arc::new(MockPlayerCounter), shutdown.clone());

    service.start_server("test").await.unwrap();
    wait_for_state(&service, ServerState::Online).await;
    service.stop_server("test").await.unwrap();
    let polls = provider.polls.load(Ordering::Acquire);
    provider.wait_for_polls(polls + 2).await;

    for _ in 0..2 {
        *provider.status.lock().await = ProviderStatus::Running;
        wait_for_state(&service, ServerState::Online).await;
        *provider.status.lock().await = ProviderStatus::Stopped;
        wait_for_state(&service, ServerState::Sleeping).await;
    }

    let result = service.ensure_started("test").await;
    let Err(ServerManagerError::StartThrottled { retry_after, .. }) = result else {
        panic!("two early exits in a row block logins, got {result:?}");
    };
    assert!(retry_after > Duration::from_secs(30), "{retry_after:?}");

    shutdown.cancel();
}
