#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(all(feature = "wasm", wasm_fixtures_available))]

mod support;

use std::time::Duration;

use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_test_harness::plugin_message::{client_message, send_to_backend, serverbound};
use infrarust_test_harness::{
    ConnectionState, FakeBackend, ProtocolVersion, ServerSpec, TestProxy,
};
use toml::Value;
use toml::value::Table;

use support::native_scripted::ScriptedPlugin;
use support::{
    add_fixture, add_precompiled_fixture, console, fresh_loader, load_enabled, make_env, read_log,
    write_script,
};

const T: Duration = infrarust_test_harness::DEFAULT_TIMEOUT;
const ECHO: &str = "test:echo";

#[tokio::test(flavor = "multi_thread")]
async fn a_named_event_round_trips_between_a_wasm_and_a_native_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    add_precompiled_fixture(&plugins_dir, "scripted").await;
    write_script(
        &plugins_dir,
        "scripted",
        "named ping late respond \"pong-from-wasm\"\ncmd ask fire hello \"from-wasm\"",
    );
    write_script(
        &plugins_dir,
        "native-peer",
        "named hello late respond \"pong-from-native\"\ncmd tell fire ping \"from-native\"",
    );
    let env = make_env(plugins_dir.clone());
    let native = ScriptedPlugin::new("native-peer");
    native
        .on_enable(env.factory.create_context("native-peer").as_ref())
        .await
        .unwrap();
    let loader = fresh_loader();
    loader.discover(&plugins_dir).await.unwrap();
    let wasm = load_enabled(&loader, &env.factory, "scripted").await;

    for line in ["ask", "tell"] {
        let dispatched = env.command_manager.dispatch(console(), line).await;
        assert_eq!(
            dispatched,
            infrarust_core::services::command_manager::DispatchOutcome::Executed,
            "{line}"
        );
    }
    wasm.on_disable().await.unwrap();

    assert_eq!(
        read_log(&plugins_dir.join("scripted")),
        [
            "enable",
            "cmd ask fired hello false pong-from-native",
            "named ping @192 native-peer text/plain from-native false -",
            "disable",
        ]
    );
    assert_eq!(
        read_log(&plugins_dir.join("native-peer")),
        [
            "enable",
            "named hello @192 scripted text/plain from-wasm false -",
            "cmd tell fired ping false pong-from-wasm",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_without_plugin_messaging_cannot_register_a_channel() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    add_precompiled_fixture(&plugins_dir, "scripted").await;
    write_script(&plugins_dir, "scripted", &format!("channel {ECHO}"));
    let env = make_env(plugins_dir.clone());
    let loader = fresh_loader();
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = loader.load("scripted", &env.factory).await.unwrap();
    let refused = plugin
        .on_enable(env.factory.create_context("scripted").as_ref())
        .await
        .expect_err("registering a channel without plugin-messaging fails on_enable");
    assert!(
        refused
            .to_string()
            .contains("missing capability: plugin-messaging"),
        "{refused}"
    );
}

fn grant_messaging(plugins_dir: std::path::PathBuf) -> impl FnOnce(&mut Table) + Send + 'static {
    move |table| {
        table.insert(
            "plugins_dir".into(),
            Value::String(plugins_dir.to_string_lossy().into_owned()),
        );
        let grants = Table::from_iter([(
            "permissions".to_owned(),
            Value::Array(vec![Value::String("plugin-messaging".into())]),
        )]);
        table.insert(
            "plugins".into(),
            Value::Table(Table::from_iter([(
                "scripted".to_owned(),
                Value::Table(grants),
            )])),
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_plugin_answers_a_client_plugin_message_through_a_running_proxy() {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    add_fixture(&plugins_dir, "scripted", "scripted");
    write_script(
        &plugins_dir,
        "scripted",
        &format!("channel {ECHO}\non plugin-message normal reply \"pong\""),
    );
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .loader(Box::new(fresh_loader()))
        .patch_config(grant_messaging(plugins_dir.clone()))
        .start()
        .await
        .unwrap();
    assert!(
        proxy.plugin_context("scripted").await.is_some(),
        "the WASM plugin is enabled"
    );

    let version = ProtocolVersion::V1_21;
    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    send_to_backend(&session, ECHO, b"ping".to_vec())
        .await
        .unwrap();
    let reply = client_message(&mut session, ECHO, T).await.unwrap();
    assert_eq!(reply.data, b"pong");

    session.chat("marker").await.unwrap();
    conn.chat_until("marker", T).await.unwrap();
    let reached: Vec<String> = conn
        .received()
        .iter()
        .filter_map(|frame| serverbound(frame, ConnectionState::Play, version).unwrap())
        .map(|message| message.channel)
        .filter(|channel| channel == ECHO)
        .collect();
    assert!(
        reached.is_empty(),
        "the plugin handled the message, so the backend never saw it: {reached:?}"
    );
    assert_eq!(
        read_log(&plugins_dir.join("scripted")),
        [
            "enable".to_owned(),
            format!("plugin-message @128 1 client {ECHO} - {ECHO} ping play"),
        ]
    );

    proxy.shutdown().await.unwrap();
}
