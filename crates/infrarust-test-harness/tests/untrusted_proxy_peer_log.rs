#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_test_harness::{FakeBackend, ProtocolVersion, ServerSpec, TestProxy};
use toml::Value;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn an_untrusted_proxy_peer_is_closed_without_a_warning() {
    let captured = Captured::default();
    let writer = captured.clone();
    let guard = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(LevelFilter::WARN)
        .with_writer(move || writer.clone())
        .finish()
        .set_default();

    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(|table| {
            table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
            table.insert(
                "trusted_proxies".into(),
                Value::Array(vec![Value::String("192.0.2.0/24".into())]),
            );
        })
        .start()
        .await
        .unwrap();

    for _ in 0..3 {
        let refused = proxy
            .client(ProtocolVersion::V1_21)
            .timeout(Duration::from_secs(2))
            .status()
            .await;
        assert!(refused.is_err(), "{refused:?}");
    }
    proxy.shutdown().await.unwrap();
    drop(guard);

    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains("trusted_proxies"), "{logs}");
}
