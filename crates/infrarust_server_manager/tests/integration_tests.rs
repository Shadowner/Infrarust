use infrarust_server_manager::{
    ApiProvider, ApiServerStatus, CrashDetector, MockApiProvider, ServerManager, ServerState,
};

use std::time::Duration;
use tokio::time;

fn create_test_manager() -> (ServerManager<MockApiProvider>, MockApiProvider) {
    let mock_api = MockApiProvider::new().with_server(
        "test-server",
        ApiServerStatus {
            id: "test-server".to_string(),
            name: "Test Server".to_string(),
            status: ServerState::Stopped,
            is_running: false,
            is_crashed: false,
            error: None,
        },
    );

    let server_manager =
        ServerManager::new(mock_api.clone()).with_check_interval(Duration::from_millis(100));

    (server_manager, mock_api)
}

#[tokio::test]
async fn test_start_server() {
    let (server_manager, mock_api) = create_test_manager();

    let initial_status = mock_api.get_server_status("test-server").await.unwrap();
    assert!(!initial_status.is_running);
    assert!(initial_status.error.is_none());

    server_manager.start_server("test-server").await.unwrap();

    let status = mock_api.get_server_status("test-server").await.unwrap();
    assert!(status.is_running);
    assert!(status.error.is_none());
    assert_eq!(status.status, ServerState::Running);
    assert!(!status.is_crashed);
}

#[tokio::test]
async fn test_stop_server() {
    let (server_manager, mock_api) = create_test_manager();

    mock_api.set_server_running("test-server");
    let initial_status = mock_api.get_server_status("test-server").await.unwrap();
    assert!(initial_status.is_running);

    server_manager.stop_server("test-server").await.unwrap();

    let status = mock_api.get_server_status("test-server").await.unwrap();
    assert!(!status.is_running);
    assert!(status.error.is_none());
    assert_eq!(status.status, ServerState::Stopped);
    assert!(!status.is_crashed);
}

#[tokio::test]
async fn test_restart_server() {
    let (server_manager, mock_api) = create_test_manager();

    mock_api.set_server_running("test-server");

    server_manager.restart_server("test-server").await.unwrap();

    let status = mock_api.get_server_status("test-server").await.unwrap();
    assert!(status.is_running);
    assert!(status.error.is_none());
    assert_eq!(status.status, ServerState::Running);
    assert!(!status.is_crashed);
}

#[tokio::test]
async fn test_crash_detection() {
    let detector = CrashDetector::new(2, Duration::from_millis(500));

    let mock_api = MockApiProvider::new().with_server(
        "crash-server",
        ApiServerStatus {
            id: "crash-server".to_string(),
            name: "Crash Test Server".to_string(),
            status: ServerState::Running,
            is_running: true,
            is_crashed: false,
            error: None,
        },
    );

    let server_manager = ServerManager::new(mock_api.clone()).with_crash_detector(detector);

    mock_api.set_server_crashed("crash-server");

    time::sleep(Duration::from_millis(100)).await;

    tokio::spawn(async move {
        let _ = server_manager.monitor_server("crash-server").await;
    });

    time::sleep(Duration::from_millis(200)).await;

    let api_status = mock_api.get_server_status("crash-server").await.unwrap();
    assert!(!api_status.is_running);
    assert!(api_status.is_crashed);
    assert_eq!(api_status.status, ServerState::Crashed);
    assert!(api_status.error.is_some());
    let error = api_status.error.unwrap();
    assert!(error.contains("crash"), "Expected crash message in error");
}

#[tokio::test]
async fn test_execute_system_command() {
    let (server_manager, _) = create_test_manager();

    #[cfg(target_os = "windows")]
    let command = "echo Hello, World!";
    #[cfg(target_os = "linux")]
    let command = "echo 'Hello, World!'";

    let result = server_manager.execute_system_command(command);
    assert!(
        result.is_ok(),
        "Command execution failed: {:?}",
        result.err()
    );

    let output = result.unwrap();
    assert!(
        output.contains("Hello, World!"),
        "Command output doesn't contain expected text"
    );
}
