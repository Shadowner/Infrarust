#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::{Arc, Mutex};

use infrarust_api::loader::PluginContextFactory;
use infrarust_api::plugin::Plugin;
use infrarust_api::types::PlayerId;
use infrarust_plugin_stats_native::StatsPlugin;

use support::make_env;
use support::mock_services::RecordingPlayerRegistry;

#[tokio::test(flavor = "multi_thread")]
async fn native_count_command_matches_wasm() {
    let tmp = tempfile::tempdir().unwrap();
    let env = make_env(tmp.path().to_path_buf());

    let ctx = env.factory.create_context("stats");
    StatsPlugin
        .on_enable(ctx.as_ref())
        .await
        .expect("native stats enables");

    let sent = Arc::new(Mutex::new(Vec::new()));
    let registry = RecordingPlayerRegistry {
        count: 7,
        sent: Arc::clone(&sent),
    };
    let found = env
        .command_manager
        .dispatch(Some(PlayerId::new(1)), "count", &registry)
        .await;
    assert!(found, "native 'count' command should be registered");

    assert_eq!(
        sent.lock().unwrap().as_slice(),
        ["Online: 7".to_string()],
        "native /count replies identically to the wasm build"
    );
}
