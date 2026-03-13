use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use infrarust_transport::forward::*;

async fn create_connected_pair() -> (tokio::net::TcpStream, tokio::net::TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();

    (client, server)
}

#[tokio::test]
async fn test_copy_forwarder_bidirectional() {
    let (client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, backend_side) = create_connected_pair().await;

    let shutdown = CancellationToken::new();
    let forwarder = CopyForwarder;

    let forward_handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            forwarder
                .forward(proxy_client, proxy_backend, shutdown)
                .await
        }
    });

    // Split client and backend for independent read/write
    let (mut client_read, mut client_write) = client_side.into_split();
    let (mut backend_read, mut backend_write) = backend_side.into_split();

    // Send data client → backend
    client_write.write_all(b"hello backend").await.unwrap();

    let mut buf = vec![0u8; 64];
    let n = backend_read.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"hello backend");

    // Send data backend → client
    backend_write.write_all(b"hello client").await.unwrap();

    let n = client_read.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"hello client");

    // Close both sides to allow copy_bidirectional to complete
    drop(client_write);
    drop(client_read);
    drop(backend_write);
    drop(backend_read);

    let result = tokio::time::timeout(Duration::from_secs(5), forward_handle)
        .await
        .unwrap()
        .unwrap();

    assert!(result.client_to_backend > 0);
}

#[tokio::test]
async fn test_copy_forwarder_shutdown() {
    let (_client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, _backend_side) = create_connected_pair().await;

    let shutdown = CancellationToken::new();
    let forwarder = CopyForwarder;

    let forward_handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            forwarder
                .forward(proxy_client, proxy_backend, shutdown)
                .await
        }
    });

    // Allow some time for forwarding to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Cancel
    shutdown.cancel();

    let result = tokio::time::timeout(Duration::from_secs(5), forward_handle)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(result.reason, ForwardEndReason::Shutdown));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_splice_forwarder_linux() {
    let (client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, backend_side) = create_connected_pair().await;

    let shutdown = CancellationToken::new();
    let forwarder = SpliceForwarder::new();

    let forward_handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            forwarder
                .forward(proxy_client, proxy_backend, shutdown)
                .await
        }
    });

    let (mut client_read, mut client_write) = client_side.into_split();
    let (mut backend_read, mut backend_write) = backend_side.into_split();

    // Send data client → backend
    client_write.write_all(b"splice test").await.unwrap();

    // Small delay for splice to transfer
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(2), backend_read.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf[..n], b"splice test");

    // Send data backend → client
    backend_write.write_all(b"splice back").await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let n = tokio::time::timeout(Duration::from_secs(2), client_read.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf[..n], b"splice back");

    // Close all sides
    drop(client_write);
    drop(client_read);
    drop(backend_write);
    drop(backend_read);

    let result = tokio::time::timeout(Duration::from_secs(5), forward_handle)
        .await
        .unwrap()
        .unwrap();

    assert!(result.client_to_backend > 0);
}
