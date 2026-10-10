use std::collections::HashMap;

use axum::http::StatusCode;
use infrarust_plugin_admin_api::log_layer::{LogBroadcast, LogEntry};

use crate::common::TestApi;

fn entry(message: &str) -> LogEntry {
    LogEntry {
        timestamp: "2026-01-01T00:00:00Z".into(),
        level: "INFO".into(),
        target: "infrarust_core".into(),
        message: message.into(),
        fields: HashMap::new(),
    }
}

#[tokio::test]
async fn log_history_is_not_found_without_a_log_broadcast() {
    let (status, body) = TestApi::new().get("/api/v1/logs/history").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn the_log_stream_is_not_found_without_a_log_broadcast() {
    let api = TestApi::new();
    let (status, _) = api.get("/api/v1/logs?token=test-key").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn log_history_serves_the_broadcast_it_was_built_with() {
    let logs = LogBroadcast::new(16, 8);
    logs.history.lock().unwrap().push_back(entry("hello"));
    let api = TestApi::builder().logs(logs).build();

    let (status, body) = api.get("/api/v1/logs/history").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"][0]["message"], "hello");
}
