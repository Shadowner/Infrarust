#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(all(feature = "wasm", wasm_fixtures_available))]

mod support;

use std::path::PathBuf;
use std::time::Duration;

use infrarust_test_harness::{FakeBackend, ProtocolVersion, ServerSpec, TestProxy};
use toml::Value;
use toml::value::Table;

use support::{add_fixture, fresh_loader};

const T: Duration = infrarust_test_harness::DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion::V1_21;
const TEMPLATE_ID: &str = "fixture-doc-hello";

fn plugins_dir(dir: PathBuf) -> impl FnOnce(&mut Table) + Send + 'static {
    move |table| {
        table.insert(
            "plugins_dir".into(),
            Value::String(dir.to_string_lossy().into_owned()),
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_template_plugin_answers_hello_in_the_players_chat_as_getting_started_says() {
    let dir = tempfile::tempdir().unwrap();
    add_fixture(dir.path(), "doc-hello", "doc-hello");
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .loader(Box::new(fresh_loader()))
        .patch_config(plugins_dir(dir.path().to_path_buf()))
        .start()
        .await
        .unwrap();
    assert!(
        proxy.plugin_context(TEMPLATE_ID).await.is_some(),
        "the template plugin is enabled under its crate name"
    );
    let mut session = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();

    session.command("hello").await.unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "hello, world!"
    );
    session.command("hello Steve").await.unwrap();
    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "hello, Steve!"
    );
}
