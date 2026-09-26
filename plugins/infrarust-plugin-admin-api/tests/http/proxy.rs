use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use axum::http::StatusCode;
use infrarust_api::services::ban_service::{BanEntry, BanSource, BanTarget};
use infrarust_api::test_util::{MockBanService, MockPlayerRegistry, MockServerManager};
use infrarust_plugin_admin_api::sse::stats_ticker::StatsTicker;
use infrarust_plugin_admin_api::state::ApiEvent;
use serde_json::json;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::common::TestApi;

fn one_expired_and_one_active_ban() -> MockBanService {
    let hour = Duration::from_secs(3600);
    let expired = BanEntry::new(
        "expired",
        BanTarget::Username("gone".into()),
        BanSource::System,
    )
    .created_at(SystemTime::now() - 2 * hour)
    .lasting(hour);
    let active = BanEntry::new(
        "active",
        BanTarget::Username("here".into()),
        BanSource::System,
    )
    .lasting(hour);
    MockBanService::new().with_entry(expired).with_entry(active)
}

#[tokio::test]
async fn test_stats_overview_returns_200() {
    let (status, body) = TestApi::new().get("/api/v1/stats").await;
    assert_eq!(status, StatusCode::OK);
    let data = &body["data"];
    assert_eq!(data["players_online"], 0);
    assert_eq!(data["servers_total"], 2);
    assert!(data["uptime_seconds"].is_u64());
    assert!(data["players_by_server"].is_object());
    assert!(data["servers_by_state"].is_object());
}

#[tokio::test]
async fn stats_overview_counts_only_the_bans_still_in_force() {
    let api = TestApi::builder()
        .ban_service(Arc::new(one_expired_and_one_active_ban()))
        .build();

    let (status, body) = api.get("/api/v1/stats").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["bans_active"], 1);
}

#[tokio::test]
async fn stats_ticks_count_only_the_bans_still_in_force() {
    let (event_tx, mut events) = broadcast::channel::<ApiEvent>(16);
    let shutdown = CancellationToken::new();
    let ticker = StatsTicker::new(
        event_tx,
        Arc::new(MockPlayerRegistry::new()),
        Arc::new(MockServerManager::new()),
        Arc::new(one_expired_and_one_active_ban()),
        Instant::now(),
        shutdown.clone(),
    );
    let running = tokio::spawn(ticker.run());

    let tick = events.recv().await.unwrap();
    shutdown.cancel();
    running.await.unwrap();
    match tick {
        ApiEvent::StatsTick { bans_active, .. } => assert_eq!(bans_active, 1),
        other => panic!("expected a stats tick, got {other:?}"),
    }
}

#[tokio::test]
async fn test_proxy_shutdown_returns_200() {
    let (status, body) = TestApi::new()
        .post("/api/v1/proxy/shutdown", json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["success"], true);
}
