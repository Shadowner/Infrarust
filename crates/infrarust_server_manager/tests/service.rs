#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use infrarust_server_manager::{
    PlayerCounter, ProviderStatus, ServerManagerError, ServerManagerService, ServerProvider,
    ServerState,
};
use tokio_util::sync::CancellationToken;

struct MockProvider {
    start_called: AtomicBool,
    stop_called: AtomicBool,
    status: tokio::sync::Mutex<ProviderStatus>,
}

impl MockProvider {
    fn new(initial: ProviderStatus) -> Self {
        Self {
            start_called: AtomicBool::new(false),
            stop_called: AtomicBool::new(false),
            status: tokio::sync::Mutex::new(initial),
        }
    }
}

impl ServerProvider for MockProvider {
    fn start(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), ServerManagerError>> + Send + '_>>
    {
        Box::pin(async move {
            self.start_called.store(true, Ordering::Release);
            *self.status.lock().await = ProviderStatus::Starting;
            Ok(())
        })
    }

    fn stop(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), ServerManagerError>> + Send + '_>>
    {
        Box::pin(async move {
            self.stop_called.store(true, Ordering::Release);
            *self.status.lock().await = ProviderStatus::Stopped;
            Ok(())
        })
    }

    fn check_status(
        &self,
    ) -> Pin<
        Box<
            dyn std::future::Future<Output = Result<ProviderStatus, ServerManagerError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move { Ok(*self.status.lock().await) })
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
    let service = ServerManagerService::new(&[], reqwest::Client::new());
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
    let service = ServerManagerService::new(&[], reqwest::Client::new());

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
