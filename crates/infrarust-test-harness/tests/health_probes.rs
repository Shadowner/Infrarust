#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_config::ServerAddress;
use infrarust_core::loadbalancer::BackendState;
use infrarust_test_harness::{FakeBackend, ServerSpec, TestProxy};
use toml::{Table, Value};

const THREE_PROBES: Duration = Duration::from_secs(8);

fn probe_every_sweep(table: &mut Table) {
    table.insert("send_proxy_protocol".into(), Value::Boolean(true));
    table.insert(
        "active_health".into(),
        Value::Table(Table::from_iter([
            ("kind".to_string(), Value::String("status_ping".into())),
            ("probe_healthy".to_string(), Value::Boolean(true)),
            ("interval".to_string(), Value::String("0s".into())),
        ])),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_probes_send_the_proxy_protocol_header_the_backend_requires() {
    let backend = FakeBackend::builder()
        .require_proxy_protocol()
        .spawn()
        .await
        .unwrap();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .patch(probe_every_sweep),
        )
        .start()
        .await
        .unwrap();
    let address: ServerAddress = backend.addr().to_string().parse().unwrap();

    tokio::time::timeout(THREE_PROBES, async {
        while backend.accepted_connections() < 3 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("three probes reach the backend");
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        backend.status_requests() >= 3,
        "every probe must complete a status exchange, got {}",
        backend.status_requests()
    );
    assert_eq!(
        proxy.services().backend_health.snapshot(&address).state,
        BackendState::Healthy,
        "a backend that requires the PROXY header stays healthy under active probing"
    );

    proxy.shutdown().await.unwrap();
}
