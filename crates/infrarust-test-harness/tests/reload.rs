#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, Recorded, Recorder, ServerSpec, TestProxy,
};
use serde_json::{Value, json};

const T: Duration = DEFAULT_TIMEOUT;

fn servers_dir(proxy: &TestProxy) -> PathBuf {
    proxy.dir().join("servers")
}

fn server_file(domain: &str) -> String {
    format!("domains = [\"{domain}\"]\naddresses = [\"127.0.0.1:1\"]\n")
}

async fn reload_after(recorder: &Recorder, seen: usize) -> Recorded {
    recorder
        .wait_for(
            |e| e.kind == EventKind::ConfigReload && recorder.count(EventKind::ConfigReload) > seen,
            T,
        )
        .await
        .unwrap();
    recorder.of(EventKind::ConfigReload)[seen].clone()
}

fn diff(added: &[&str], removed: &[&str], updated: &[&str]) -> Value {
    json!({
        "provider": "file",
        "added": added,
        "removed": removed,
        "updated": updated,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_batch_of_file_changes_is_one_reload_with_its_diff() {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("alpha").unreachable())
        .server(ServerSpec::offline("beta").unreachable())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    tokio::time::timeout(T, proxy.bus().flush()).await.unwrap();
    assert_eq!(recorder.count(EventKind::ConfigReload), 0);
    let dir = servers_dir(&proxy);

    std::fs::write(dir.join("gamma.toml"), server_file("gamma.test")).unwrap();
    std::fs::write(dir.join("delta.toml"), server_file("delta.test")).unwrap();
    std::fs::remove_file(dir.join("alpha.toml")).unwrap();
    std::fs::write(dir.join("beta.toml"), server_file("beta.example")).unwrap();

    let batch = reload_after(&recorder, 0).await;
    assert_eq!(
        batch.detail,
        diff(&["delta", "gamma"], &["alpha"], &["beta"])
    );
    let router = &proxy.services().domain_router;
    assert!(router.find_by_server_id("alpha").is_none());
    assert_eq!(
        router.find_by_server_id("beta").unwrap().domains,
        ["beta.example"]
    );

    std::fs::write(dir.join("epsilon.toml"), server_file("epsilon.test")).unwrap();
    let sentinel = reload_after(&recorder, 1).await;
    assert_eq!(sentinel.detail, diff(&["epsilon"], &[], &[]));
    tokio::time::timeout(T, proxy.bus().flush()).await.unwrap();
    assert_eq!(recorder.count(EventKind::ConfigReload), 2);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rewriting_a_server_file_with_the_same_content_reloads_nothing() {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("alpha").unreachable())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let dir = servers_dir(&proxy);
    let alpha = dir.join("alpha.toml");

    let content = std::fs::read(&alpha).unwrap();
    std::fs::write(&alpha, &content).unwrap();
    std::fs::write(&alpha, &content).unwrap();
    std::fs::write(dir.join("sentinel.toml"), server_file("sentinel.test")).unwrap();

    let reload = reload_after(&recorder, 0).await;
    assert_eq!(reload.detail, diff(&["sentinel"], &[], &[]));
    tokio::time::timeout(T, proxy.bus().flush()).await.unwrap();
    assert_eq!(recorder.count(EventKind::ConfigReload), 1);

    proxy.shutdown().await.unwrap();
}
