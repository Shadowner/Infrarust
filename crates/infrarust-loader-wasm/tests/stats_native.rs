#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use infrarust_api::loader::PluginContextFactory;
use infrarust_api::plugin::Plugin;
use infrarust_api::test_util::{MockPlayer, MockPlayerRegistry, player_source};
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_plugin_stats_native::StatsPlugin;

use support::{EnvOptions, make_env_with};

#[tokio::test(flavor = "multi_thread")]
async fn native_count_command_matches_wasm() {
    let tmp = tempfile::tempdir().unwrap();
    let player = MockPlayer::new(1, "tester").online_mode(true).into_arc();
    let registry = MockPlayerRegistry::new()
        .with(Arc::clone(&player))
        .fake_online_count(7);
    let env = make_env_with(
        tmp.path().to_path_buf(),
        EnvOptions {
            player_registry: Arc::new(registry),
            ..EnvOptions::default()
        },
    );

    let ctx = env.factory.create_context("stats");
    StatsPlugin
        .on_enable(ctx.as_ref())
        .await
        .expect("native stats enables");

    let outcome = env
        .command_manager
        .dispatch(player_source(&player), "count")
        .await;
    assert_eq!(
        outcome,
        DispatchOutcome::Executed,
        "native 'count' command should be registered"
    );

    assert_eq!(
        player.sent_text(),
        "Online: 7",
        "native /count replies identically to the wasm build"
    );
}
