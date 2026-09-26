#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;

use infrarust_config::KeepaliveConfig;
use infrarust_transport::TransportError;
use infrarust_transport::listener::{Listener, ListenerConfig};
use infrarust_transport::proxy_protocol::HEADER_TIMEOUT;

const HEADER: &[u8] = b"PROXY TCP4 203.0.113.7 127.0.0.1 50001 25565\r\n";

fn test_config() -> ListenerConfig {
    ListenerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        max_connections: 0,
        keepalive: KeepaliveConfig::default(),
        so_reuseport: false,
        receive_proxy_protocol: false,
    }
}

fn proxy_protocol_config(max_connections: u32) -> ListenerConfig {
    ListenerConfig {
        max_connections,
        receive_proxy_protocol: true,
        ..test_config()
    }
}

fn header_source() -> Option<IpAddr> {
    Some("203.0.113.7".parse().unwrap())
}

async fn send_header_then(addr: SocketAddr, payload: &[u8]) -> TcpStream {
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(&[HEADER, payload].concat()).await.unwrap();
    client
}

#[tokio::test]
async fn test_bind_and_accept() {
    let shutdown = CancellationToken::new();
    let listener = Listener::bind(test_config(), shutdown.clone())
        .await
        .unwrap();

    let addr = listener.local_addr().unwrap();

    let client = TcpStream::connect(addr).await.unwrap();
    let pending = listener.accept().await.unwrap();
    assert_eq!(pending.peer_addr(), client.local_addr().unwrap());

    let accepted = pending.resolve().await.unwrap();
    assert_eq!(
        accepted.connection.peer_addr(),
        client.local_addr().unwrap()
    );
    assert_eq!(accepted.connection.real_ip(), None);
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
    let _client1 = TcpStream::connect(addr).await.unwrap();
    let accepted1 = listener.accept().await.unwrap();

    // Second connection: the client TCP connect will succeed (kernel backlog)
    // but accept() will block on the semaphore
    let _client2 = TcpStream::connect(addr).await.unwrap();

    let accept_result = timeout(Duration::from_millis(200), listener.accept()).await;
    // Should time out because semaphore is full
    assert!(accept_result.is_err());
    drop(accept_result);

    // Drop first connection to release semaphore
    drop(accepted1);

    // Now second accept should succeed
    let _accepted2 = timeout(Duration::from_secs(2), listener.accept())
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

#[tokio::test]
async fn accept_does_not_wait_for_the_proxy_protocol_header() {
    let shutdown = CancellationToken::new();
    let listener = Listener::bind(proxy_protocol_config(0), shutdown.clone())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let silent = TcpStream::connect(addr).await.unwrap();
    let _talker = send_header_then(addr, b"hello").await;

    let waiting = timeout(Duration::from_secs(1), listener.accept())
        .await
        .expect("accept waited for the silent client's header")
        .unwrap();
    assert_eq!(waiting.peer_addr(), silent.local_addr().unwrap());

    let pending = timeout(Duration::from_secs(1), listener.accept())
        .await
        .expect("the next connection is accepted")
        .unwrap();
    let mut accepted = timeout(Duration::from_secs(1), pending.resolve())
        .await
        .expect("the header already arrived")
        .unwrap();
    assert_eq!(accepted.connection.real_ip(), header_source());
    assert_eq!(accepted.connection.peek(5).await.unwrap(), b"hello");

    drop(waiting);
    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn a_silent_connection_holds_its_permit_until_its_header_times_out() {
    let shutdown = CancellationToken::new();
    let listener = Listener::bind(proxy_protocol_config(2), shutdown.clone())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let mut silent = TcpStream::connect(addr).await.unwrap();
    let waiting = listener.accept().await.unwrap();
    let _talker = send_header_then(addr, b"hello").await;
    let talking = listener.accept().await.unwrap();
    let _late = send_header_then(addr, b"").await;

    let full = timeout(Duration::from_secs(1), listener.accept()).await;
    assert!(full.is_err(), "both permits are held, got {full:?}");
    drop(full);

    let started = Instant::now();
    let silent_result = tokio::spawn(waiting.resolve());
    let mut accepted = talking.resolve().await.unwrap();
    assert_eq!(accepted.connection.real_ip(), header_source());

    let err = silent_result.await.unwrap().unwrap_err();
    assert!(
        matches!(&err, TransportError::ProxyProtocolDecode(msg) if msg.contains("timed out")),
        "{err:?}"
    );
    let waited = started.elapsed();
    assert!(waited >= HEADER_TIMEOUT, "{waited:?}");
    let mut byte = [0u8; 1];
    let read = silent.read(&mut byte).await.unwrap();
    assert_eq!(read, 0, "the silent connection is closed");

    let late = timeout(Duration::from_secs(1), listener.accept())
        .await
        .expect("the timed-out connection gave its permit back")
        .unwrap();
    assert_eq!(
        late.resolve().await.unwrap().connection.real_ip(),
        header_source()
    );
    assert_eq!(accepted.connection.peek(5).await.unwrap(), b"hello");

    shutdown.cancel();
}
