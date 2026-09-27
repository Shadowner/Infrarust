use axum::http::StatusCode;

use crate::common::TestApi;

#[tokio::test]
async fn test_plugins_list_returns_200() {
    let (status, body) = TestApi::new().get("/api/v1/plugins").await;
    assert_eq!(status, StatusCode::OK);
    let plugins = body["data"].as_array().unwrap();
    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0]["id"], "admin_api");
    assert_eq!(plugins[0]["state"], "enabled");
    assert!(plugins[0]["runtime"].is_null());
    assert_eq!(plugins[1]["runtime"]["health"], "quarantined");
}

#[tokio::test]
async fn test_plugins_get_returns_200() {
    let (status, body) = TestApi::new().get("/api/v1/plugins/admin_api").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["id"], "admin_api");
    assert_eq!(body["data"]["version"], "0.1.0");
}

#[tokio::test]
async fn test_plugins_get_returns_404_for_unknown() {
    let (status, body) = TestApi::new().get("/api/v1/plugins/nonexistent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn a_quarantined_plugin_stays_enabled_and_carries_its_runtime_health() {
    let (status, body) = TestApi::new().get("/api/v1/plugins/flaky").await;
    assert_eq!(status, StatusCode::OK);
    let plugin = &body["data"];
    assert_eq!(plugin["state"], "enabled");
    let runtime = &plugin["runtime"];
    assert_eq!(runtime["health"], "quarantined");
    assert_eq!(runtime["retry_in_ms"], 12_500);
    assert_eq!(runtime["generation"], 7);
    assert_eq!(runtime["restarts_in_window"], 5);
    assert_eq!(runtime["max_restarts"], 5);
    assert_eq!(runtime["restart_window_secs"], 300);
    assert_eq!(
        runtime["last_fault"]["cause"],
        "the call ran past the event deadline"
    );
    assert_eq!(runtime["last_fault"]["secs_ago"], 4);
    assert_eq!(runtime["last_fault"]["generation"], 7);
    let queue = &runtime["queue"];
    assert_eq!(queue["depth"], 3);
    assert_eq!(queue["capacity"], 1024);
    assert_eq!(queue["window_secs"], 60);
    assert_eq!(queue["taken"], 1234);
    assert_eq!(queue["peak_depth"], 12);
    assert_eq!(queue["wait_p50_us"], 21);
    assert_eq!(queue["wait_p99_us"], 1_250);
    assert_eq!(queue["wait_max_us"], 3_000);
}
