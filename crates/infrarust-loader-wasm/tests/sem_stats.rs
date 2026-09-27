#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::event::BoxFuture;
use infrarust_api::events::lifecycle::{DisconnectCause, DisconnectEvent, PostLoginEvent};
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::types::ServerId;
use infrarust_plugin_stats_native::StatsPlugin;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{TestEnv, fresh_loader, make_env, stage};

fn steve() -> std::sync::Arc<dyn infrarust_api::player::Player> {
    support::session_player(
        1,
        support::nil_profile("Steve"),
        767,
        "203.0.113.7:40000".parse().unwrap(),
    )
}

async fn join_and_leave(env: &TestEnv) {
    env.event_bus.fire(PostLoginEvent::new(steve())).await;
    env.event_bus
        .fire(DisconnectEvent::new(
            steve(),
            Some(ServerId::new("lobby")),
            DisconnectCause::ClientQuit,
        ))
        .await;
}

fn stats_lines(logs: &LogCapture) -> Vec<String> {
    let mut lines: Vec<String> = ["[stats] Steve joined", "[stats] Steve left"]
        .into_iter()
        .filter(|needle| !logs.matching(needle).is_empty())
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

#[tokio::test(flavor = "multi_thread")]
async fn the_wasm_stats_plugin_logs_joins_and_leaves_like_the_native_build() {
    let native_logs = LogCapture::at(Level::INFO);
    async {
        let tmp = tempfile::tempdir().unwrap();
        let env = make_env(tmp.path().to_path_buf());
        let ctx = env.factory.create_context("stats");
        StatsPlugin.on_enable(ctx.as_ref()).await.unwrap();
        join_and_leave(&env).await;
    }
    .with_subscriber(native_logs.clone())
    .await;

    let wasm_logs = LogCapture::at(Level::INFO);
    async {
        let (_tmp, plugins_dir) = stage("stats");
        let env = make_env(plugins_dir.clone());
        let loader = fresh_loader();
        loader.discover(&plugins_dir).await.unwrap();
        let _plugin = support::load_enabled(&loader, &env.factory, "stats").await;
        join_and_leave(&env).await;
    }
    .with_subscriber(wasm_logs.clone())
    .await;

    assert_eq!(
        stats_lines(&native_logs),
        ["[stats] Steve joined", "[stats] Steve left"]
    );
    assert_eq!(stats_lines(&wasm_logs), stats_lines(&native_logs));
}

struct Taken;

impl CommandHandler for Taken {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

fn take_count(env: &TestEnv) {
    env.command_manager
        .register_owned("other", CommandSpec::new("count"), Box::new(Taken))
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_wasm_stats_plugin_still_enables_when_its_command_name_is_taken_like_the_native_build()
{
    let native_logs = LogCapture::at(Level::INFO);
    async {
        let tmp = tempfile::tempdir().unwrap();
        let native_env = make_env(tmp.path().to_path_buf());
        take_count(&native_env);
        let ctx = native_env.factory.create_context("stats");
        assert!(
            StatsPlugin.on_enable(ctx.as_ref()).await.is_ok(),
            "the native build warns and keeps its listeners"
        );
        join_and_leave(&native_env).await;
    }
    .with_subscriber(native_logs.clone())
    .await;

    let wasm_logs = LogCapture::at(Level::INFO);
    async {
        let (_tmp, plugins_dir) = stage("stats");
        let env = make_env(plugins_dir.clone());
        take_count(&env);
        let loader = fresh_loader();
        loader.discover(&plugins_dir).await.unwrap();
        let plugin = loader.load("stats", &env.factory).await.unwrap();
        let ctx = env.factory.create_context("stats");
        let enabled = plugin.on_enable(ctx.as_ref()).await;
        assert!(
            enabled.is_ok(),
            "the WASM build of the same plugin refuses to enable when `count` is taken: {enabled:?}"
        );
        join_and_leave(&env).await;
    }
    .with_subscriber(wasm_logs.clone())
    .await;

    for logs in [&native_logs, &wasm_logs] {
        assert_eq!(
            logs.matching("[stats] /count was not registered").len(),
            1,
            "{:?}",
            logs.lines()
        );
    }
    assert_eq!(
        stats_lines(&native_logs),
        ["[stats] Steve joined", "[stats] Steve left"]
    );
    assert_eq!(stats_lines(&wasm_logs), stats_lines(&native_logs));
}
