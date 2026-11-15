#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::time::Duration;

use infrarust_test_harness::{FakeBackend, ProtocolVersion, ServerSpec, TestProxy};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, timeout};
use toml::{Table, Value};

const VERSION: ProtocolVersion = ProtocolVersion::V1_21;
const PROMPTLY: Duration = Duration::from_secs(2);
const STILL_OPEN_AFTER: Duration = Duration::from_secs(1);

fn per_ip_limit(limit: i64) -> impl FnOnce(&mut Table) + Send + 'static {
    move |table| {
        table.insert("max_connections_per_ip".into(), Value::Integer(limit));
    }
}

async fn is_closed_by_the_proxy(mut stream: TcpStream) -> bool {
    let mut byte = [0u8; 1];
    timeout(STILL_OPEN_AFTER, stream.read(&mut byte))
        .await
        .is_ok()
}

async fn closed_count(streams: Vec<TcpStream>) -> usize {
    let mut reads: JoinSet<bool> = streams.into_iter().map(is_closed_by_the_proxy).collect();
    let mut closed = 0;
    while let Some(read) = reads.join_next().await {
        if read.unwrap() {
            closed += 1;
        }
    }
    closed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_33rd_connection_from_one_address_is_closed() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(per_ip_limit(32))
        .start()
        .await
        .unwrap();

    let mut streams = Vec::new();
    for _ in 0..33 {
        streams.push(TcpStream::connect(proxy.addr()).await.unwrap());
    }
    assert_eq!(closed_count(streams).await, 1);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn addresses_in_one_ipv6_64_share_the_limit() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(|table| {
            per_ip_limit(1)(table);
            table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
        })
        .start()
        .await
        .unwrap();

    let mut same_64 = Vec::new();
    for source in ["2001:db8:1:1::1", "2001:db8:1:1::2"] {
        let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
        let header = format!("PROXY TCP6 {source} ::1 50001 25565\r\n");
        stream.write_all(header.as_bytes()).await.unwrap();
        same_64.push(stream);
    }
    let mut other_64 = TcpStream::connect(proxy.addr()).await.unwrap();
    other_64
        .write_all(b"PROXY TCP6 2001:db8:1:2::1 ::1 50001 25565\r\n")
        .await
        .unwrap();

    assert_eq!(closed_count(same_64).await, 1);
    assert!(!is_closed_by_the_proxy(other_64).await);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_slot_is_held_during_a_forward_and_freed_when_it_ends() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("lobby").backend(backend.addr()))
        .patch_config(per_ip_limit(1))
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();

    let refused = proxy.client(VERSION).timeout(PROMPTLY).status().await;
    assert!(refused.is_err(), "the forward holds the only slot");

    let mut backend_conn = backend.next_connection(PROMPTLY).await.unwrap();
    session.quit().await;
    backend_conn.close().await;

    let deadline = Instant::now() + PROMPTLY;
    loop {
        if proxy
            .client(VERSION)
            .timeout(PROMPTLY)
            .status()
            .await
            .is_ok()
        {
            break;
        }
        assert!(Instant::now() < deadline, "the slot was not freed");
        sleep(Duration::from_millis(20)).await;
    }

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_harness_disables_the_per_ip_limit() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();
    let addr: SocketAddr = proxy.addr();

    let mut streams = Vec::new();
    for _ in 0..33 {
        streams.push(TcpStream::connect(addr).await.unwrap());
    }
    assert_eq!(closed_count(streams).await, 0);

    proxy.shutdown().await.unwrap();
}
