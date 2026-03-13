use std::time::Duration;

use tokio_util::sync::CancellationToken;

use infrarust_config::KeepaliveConfig;
use infrarust_transport::listener::{Listener, ListenerConfig};

fn test_config() -> ListenerConfig {
    ListenerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        max_connections: 0,
        keepalive: KeepaliveConfig::default(),
        so_reuseport: false,
        receive_proxy_protocol: false,
    }
}

#[tokio::test]
async fn test_bind_and_accept() {
    let shutdown = CancellationToken::new();
    let listener = Listener::bind(test_config(), shutdown.clone())
        .await
        .unwrap();

    let addr = listener.local_addr().unwrap();

    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let accepted = listener.accept().await.unwrap();

    assert_eq!(
        accepted.connection.peer_addr(),
        client.local_addr().unwrap()
    );
    drop(accepted);
    shutdown.cancel();
}

#[tokio::test]
async fn test_max_connections_semaphore() {
    let shutdown = CancellationToken::new();
    let mut config = test_config();
    config.max_connections = 1;

    let listener = Listener::bind(config, shutdown.clone()).await.unwrap();
    let addr = listener.local_addr().unwrap();

    // First connection should succeed
    let _client1 = tokio::net::TcpStream::connect(addr).await.unwrap();
    let accepted1 = listener.accept().await.unwrap();

    // Second connection: the client TCP connect will succeed (kernel backlog)
    // but accept() will block on the semaphore
    let _client2 = tokio::net::TcpStream::connect(addr).await.unwrap();

    let accept_result = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    // Should time out because semaphore is full
    assert!(accept_result.is_err());
    drop(accept_result);

    // Drop first connection to release semaphore
    drop(accepted1);

    // Now second accept should succeed
    let _accepted2 = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();

    shutdown.cancel();
}

#[tokio::test]
async fn test_shutdown_stops_accept() {
    let shutdown = CancellationToken::new();
    let listener = Listener::bind(test_config(), shutdown.clone())
        .await
        .unwrap();

    shutdown.cancel();

    let result = listener.accept().await;
    assert!(result.is_err());
    drop(result);
}
