use std::net::SocketAddr;

use infrarust_config::CraftyManagerConfig;
use infrarust_server_manager::{CraftyProvider, ProviderStatus, ServerProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// Spawns a mock HTTP server that returns canned responses based on path matching.
async fn spawn_mock_http(
    responses: Vec<(&'static str, u16, &'static str)>,
) -> (SocketAddr, CancellationToken) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (mut stream, _) = result.unwrap();
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap();
                    let request = String::from_utf8_lossy(&buf[..n]);

                    let first_line = request.lines().next().unwrap_or("");

                    let (status, body) = responses
                        .iter()
                        .find(|(path_contains, _, _)| first_line.contains(path_contains))
                        .map(|(_, status, body)| (*status, *body))
                        .unwrap_or((404, r#"{"error": "not found"}"#));

                    let response = format!(
                        "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        status,
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.flush().await;
                }
                _ = token.cancelled() => break,
            }
        }
    });

    (addr, shutdown)
}

fn make_config(addr: SocketAddr) -> CraftyManagerConfig {
    CraftyManagerConfig {
        api_url: format!("http://{}", addr),
        api_key: "test-api-key".to_string(),
        server_id: "crafty-uuid-123".to_string(),
        shutdown_after: None,
        start_timeout: std::time::Duration::from_secs(10),
        poll_interval: std::time::Duration::from_secs(5),
    }
}

#[tokio::test]
async fn test_check_status_running() {
    let (addr, shutdown) = spawn_mock_http(vec![(
        "/stats",
        200,
        r#"{"status":"ok","data":{"running":true}}"#,
    )])
    .await;

    let config = make_config(addr);
    let provider = CraftyProvider::new(&config, reqwest::Client::new());

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Running);

    shutdown.cancel();
}

#[tokio::test]
async fn test_check_status_stopped() {
    let (addr, shutdown) = spawn_mock_http(vec![(
        "/stats",
        200,
        r#"{"status":"ok","data":{"running":false}}"#,
    )])
    .await;

    let config = make_config(addr);
    let provider = CraftyProvider::new(&config, reqwest::Client::new());

    let status = provider.check_status().await.unwrap();
    assert_eq!(status, ProviderStatus::Stopped);

    shutdown.cancel();
}

#[tokio::test]
async fn test_start_sends_action() {
    let (addr, shutdown) = spawn_mock_http(vec![("start_server", 200, r#"{"status":"ok"}"#)]).await;

    let config = make_config(addr);
    let provider = CraftyProvider::new(&config, reqwest::Client::new());

    provider.start().await.unwrap();

    shutdown.cancel();
}

#[tokio::test]
async fn test_stop_sends_action() {
    let (addr, shutdown) = spawn_mock_http(vec![("stop_server", 200, r#"{"status":"ok"}"#)]).await;

    let config = make_config(addr);
    let provider = CraftyProvider::new(&config, reqwest::Client::new());

    provider.stop().await.unwrap();

    shutdown.cancel();
}

#[tokio::test]
async fn test_api_error() {
    let (addr, shutdown) = spawn_mock_http(vec![("/stats", 500, r#"{"error": "internal"}"#)]).await;

    let config = make_config(addr);
    let provider = CraftyProvider::new(&config, reqwest::Client::new());

    let result = provider.check_status().await;
    assert!(result.is_err());

    shutdown.cancel();
}
