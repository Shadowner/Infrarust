#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;

use infrarust_test_harness::{FakeBackend, ProtocolVersion, ServerSpec, TestProxy};
use tokio::net::TcpStream;
use tokio::time::Instant;
use toml::Value;

const VERSION: ProtocolVersion = ProtocolVersion::V1_21;
const PROMPTLY: Duration = Duration::from_secs(2);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_never_sends_its_proxy_header_does_not_hold_up_the_next_one() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(|table| {
            table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
        })
        .start()
        .await
        .unwrap();
    let silent = TcpStream::connect(proxy.addr()).await.unwrap();
    let source: SocketAddr = "203.0.113.7:50001".parse().unwrap();

    let started = Instant::now();
    proxy
        .client(VERSION)
        .proxy_protocol(source)
        .timeout(PROMPTLY)
        .status()
        .await
        .expect("the ping is answered while the silent connection waits for its header");
    assert!(started.elapsed() < PROMPTLY, "{:?}", started.elapsed());

    let mut byte = [0u8; 1];
    let pending = silent.try_read(&mut byte);
    assert!(
        matches!(&pending, Err(e) if e.kind() == ErrorKind::WouldBlock),
        "the silent connection is still open and waiting, got {pending:?}"
    );

    proxy.shutdown().await.unwrap();
}
