#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::{Arc, Mutex};

use infrarust_api::command::CommandSource;
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::plugin::Plugin;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::PlayerId;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_plugin_stats_native::StatsPlugin;

use support::mock_services::RecordingPlayerRegistry;
use support::{EnvOptions, make_env_with};

#[tokio::test(flavor = "multi_thread")]
async fn native_count_command_matches_wasm() {
    let tmp = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(RecordingPlayerRegistry {
        count: 7,
        sent: Arc::clone(&sent),
    });
    let env = make_env_with(
        tmp.path().to_path_buf(),
        EnvOptions {
            player_registry: Arc::clone(&registry) as _,
            ..EnvOptions::default()
        },
    );

    let ctx = env.factory.create_context("stats");
    StatsPlugin
        .on_enable(ctx.as_ref())
        .await
        .expect("native stats enables");

    let player = registry
        .get_player_by_id(PlayerId::new(1))
        .expect("the recording registry knows every id");
    let outcome = env
        .command_manager
        .dispatch(CommandSource::Player(player), "count")
        .await;
    assert_eq!(
        outcome,
        DispatchOutcome::Executed,
        "native 'count' command should be registered"
    );

    assert_eq!(
        sent.lock().unwrap().as_slice(),
        ["Online: 7".to_string()],
        "native /count replies identically to the wasm build"
    );
}
