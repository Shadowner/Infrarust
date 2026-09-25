#![allow(clippy::unwrap_used, clippy::expect_used)]
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

    // Close both sides to let the copy tasks complete
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

async fn assert_client_eof_half_close<F: Forwarder + 'static>(forwarder: F) {
    let (client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, backend_side) = create_connected_pair().await;

    let shutdown = CancellationToken::new();
    let forward_handle = tokio::spawn(async move {
        forwarder
            .forward(proxy_client, proxy_backend, shutdown)
            .await
    });

    let (mut client_read, mut client_write) = client_side.into_split();
    let (mut backend_read, mut backend_write) = backend_side.into_split();

    let request = b"ping from client";
    client_write.write_all(request).await.unwrap();
    client_write.shutdown().await.unwrap();

    let mut received = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        backend_read.read_to_end(&mut received),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(received, request, "backend should see request then EOF");

    let reply = b"pong from backend";
    backend_write.write_all(reply).await.unwrap();
    backend_write.shutdown().await.unwrap();

    let mut returned = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        client_read.read_to_end(&mut returned),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(returned, reply, "client should still receive the reply");

    let result = tokio::time::timeout(Duration::from_secs(5), forward_handle)
        .await
        .unwrap()
        .unwrap();

    assert!(
        matches!(result.reason, ForwardEndReason::ClientClosed),
        "expected ClientClosed, got {:?}",
        result.reason
    );
    assert_eq!(result.client_to_backend, request.len() as u64);
    assert_eq!(result.backend_to_client, reply.len() as u64);
}

#[tokio::test]
async fn test_copy_forwarder_client_eof_half_close() {
    assert_client_eof_half_close(CopyForwarder).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_splice_forwarder_client_eof_half_close() {
    assert_client_eof_half_close(SpliceForwarder::new()).await;
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

const T: Duration = Duration::from_secs(5);
const SHUTDOWN_ROUNDS: usize = 20;

type Forwarding<'a> = std::pin::Pin<Box<dyn Future<Output = ForwardResult> + Send + 'a>>;

async fn start_without_driving(forward: &mut Forwarding<'_>) {
    let pending =
        std::future::poll_fn(|cx| std::task::Poll::Ready(forward.as_mut().poll(cx).is_pending()))
            .await;
    assert!(pending, "the forward ended before either side closed");
}

async fn eof(half: &mut tokio::net::tcp::OwnedReadHalf) {
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(T, half.read(&mut byte))
        .await
        .expect("the proxy never closed this side")
        .unwrap();
    assert_eq!(read, 0);
}

async fn assert_proxy_let_go(half: &mut tokio::net::tcp::OwnedWriteHalf) {
    tokio::time::timeout(T, async {
        while half.write_all(b"x").await.is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the proxy kept its socket open");
}

async fn relay_one_byte_each_way(
    client: &mut tokio::net::TcpStream,
    backend: &mut tokio::net::TcpStream,
) {
    let mut byte = [0u8; 1];
    client.write_all(b"c").await.unwrap();
    tokio::time::timeout(T, backend.read_exact(&mut byte))
        .await
        .unwrap()
        .unwrap();
    backend.write_all(b"b").await.unwrap();
    tokio::time::timeout(T, client.read_exact(&mut byte))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn copy_forwarder_blames_the_backend_that_closed_first_even_when_polled_late() {
    let (client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, backend_side) = create_connected_pair().await;
    let forwarder = CopyForwarder;
    let mut forward = forwarder.forward(proxy_client, proxy_backend, CancellationToken::new());
    start_without_driving(&mut forward).await;
    let (mut client_read, mut client_write) = client_side.into_split();
    let (mut backend_read, mut backend_write) = backend_side.into_split();

    backend_write.shutdown().await.unwrap();
    eof(&mut client_read).await;
    client_write.shutdown().await.unwrap();
    eof(&mut backend_read).await;
    let result = tokio::time::timeout(T, forward).await.unwrap();

    assert!(
        matches!(result.reason, ForwardEndReason::BackendClosed),
        "{:?}",
        result.reason
    );
}

#[tokio::test]
async fn copy_forwarder_blames_the_client_that_closed_first_even_when_polled_late() {
    let (client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, backend_side) = create_connected_pair().await;
    let forwarder = CopyForwarder;
    let mut forward = forwarder.forward(proxy_client, proxy_backend, CancellationToken::new());
    start_without_driving(&mut forward).await;
    let (mut client_read, mut client_write) = client_side.into_split();
    let (mut backend_read, mut backend_write) = backend_side.into_split();

    client_write.shutdown().await.unwrap();
    eof(&mut backend_read).await;
    backend_write.shutdown().await.unwrap();
    eof(&mut client_read).await;
    let result = tokio::time::timeout(T, forward).await.unwrap();

    assert!(
        matches!(result.reason, ForwardEndReason::ClientClosed),
        "{:?}",
        result.reason
    );
}

async fn assert_blames_the_side_that_closed_first<F: Forwarder + Default + 'static>(
    backend_first: bool,
) {
    let (client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, backend_side) = create_connected_pair().await;
    let forward = tokio::spawn(async move {
        F::default()
            .forward(proxy_client, proxy_backend, CancellationToken::new())
            .await
    });
    let (mut client_read, mut client_write) = client_side.into_split();
    let (mut backend_read, mut backend_write) = backend_side.into_split();

    if backend_first {
        backend_write.shutdown().await.unwrap();
        eof(&mut client_read).await;
        client_write.shutdown().await.unwrap();
        eof(&mut backend_read).await;
    } else {
        client_write.shutdown().await.unwrap();
        eof(&mut backend_read).await;
        backend_write.shutdown().await.unwrap();
        eof(&mut client_read).await;
    }
    let result = tokio::time::timeout(T, forward).await.unwrap().unwrap();

    let expected = if backend_first {
        matches!(result.reason, ForwardEndReason::BackendClosed)
    } else {
        matches!(result.reason, ForwardEndReason::ClientClosed)
    };
    assert!(
        expected,
        "backend_first={backend_first}: {:?}",
        result.reason
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn splice_forwarder_blames_the_backend_that_closed_first() {
    assert_blames_the_side_that_closed_first::<SpliceForwarder>(true).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn splice_forwarder_blames_the_client_that_closed_first() {
    assert_blames_the_side_that_closed_first::<SpliceForwarder>(false).await;
}

async fn assert_shutdown_ends_a_half_closed_forward<F: Forwarder + Default + 'static>() {
    let (client_side, proxy_client) = create_connected_pair().await;
    let (proxy_backend, backend_side) = create_connected_pair().await;
    let shutdown = CancellationToken::new();
    let forward = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            F::default()
                .forward(proxy_client, proxy_backend, shutdown)
                .await
        }
    });
    let (mut client_read, mut client_write) = client_side.into_split();
    let (mut backend_read, mut backend_write) = backend_side.into_split();

    client_write.shutdown().await.unwrap();
    eof(&mut backend_read).await;
    backend_write.write_all(b"still open").await.unwrap();
    let mut relayed = [0u8; 10];
    tokio::time::timeout(T, client_read.read_exact(&mut relayed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&relayed, b"still open");

    shutdown.cancel();
    let result = tokio::time::timeout(T, forward)
        .await
        .expect("a half-closed forward must end on shutdown")
        .unwrap();

    assert!(
        matches!(result.reason, ForwardEndReason::ClientClosed),
        "{:?}",
        result.reason
    );
    assert_eq!(result.client_to_backend, 0);
    eof(&mut client_read).await;
    assert_proxy_let_go(&mut backend_write).await;
}

#[tokio::test]
async fn copy_forwarder_shutdown_ends_a_half_closed_forward() {
    assert_shutdown_ends_a_half_closed_forward::<CopyForwarder>().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn splice_forwarder_shutdown_ends_a_half_closed_forward() {
    assert_shutdown_ends_a_half_closed_forward::<SpliceForwarder>().await;
}

async fn assert_shutdown_is_reported_as_shutdown<F: Forwarder + Default + 'static>() {
    for round in 0..SHUTDOWN_ROUNDS {
        let (mut client_side, proxy_client) = create_connected_pair().await;
        let (proxy_backend, mut backend_side) = create_connected_pair().await;
        let shutdown = CancellationToken::new();
        let forward = tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                F::default()
                    .forward(proxy_client, proxy_backend, shutdown)
                    .await
            }
        });
        relay_one_byte_each_way(&mut client_side, &mut backend_side).await;

        shutdown.cancel();
        let result = tokio::time::timeout(T, forward).await.unwrap().unwrap();

        assert!(
            matches!(result.reason, ForwardEndReason::Shutdown),
            "round {round}: {:?}",
            result.reason
        );
        let (mut client_read, _client_write) = client_side.into_split();
        let (mut backend_read, _backend_write) = backend_side.into_split();
        eof(&mut client_read).await;
        eof(&mut backend_read).await;
    }
}

#[tokio::test]
async fn copy_forwarder_reports_shutdown_as_shutdown() {
    assert_shutdown_is_reported_as_shutdown::<CopyForwarder>().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn splice_forwarder_reports_shutdown_as_shutdown() {
    assert_shutdown_is_reported_as_shutdown::<SpliceForwarder>().await;
}
