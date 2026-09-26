use axum::http::StatusCode;
use serde_json::json;

use crate::common::TestApi;

#[tokio::test]
async fn test_players_list_returns_200_with_empty_list() {
    let (status, body) = TestApi::new().get("/api/v1/players").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"].is_array());
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
    assert_eq!(body["meta"]["total"], 0);
    assert_eq!(body["meta"]["page"], 1);
    assert_eq!(body["meta"]["per_page"], 20);
    assert_eq!(body["meta"]["total_pages"], 1);
}

#[tokio::test]
async fn test_players_count_returns_200() {
    let (status, body) = TestApi::new().get("/api/v1/players/count").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["total"], 0);
    assert!(body["data"]["by_server"].is_object());
    assert!(body["data"]["by_mode"].is_object());
}

#[tokio::test]
async fn test_players_get_returns_404_for_unknown() {
    let (status, body) = TestApi::new().get("/api/v1/players/nonexistent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn test_kick_player_not_found() {
    let (status, body) = TestApi::new()
        .post("/api/v1/players/unknown/kick", json!({"reason": "test"}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn test_send_player_not_found() {
    let (status, _) = TestApi::new()
        .post("/api/v1/players/unknown/send", json!({"server": "lobby"}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_message_player_not_found() {
    let (status, _) = TestApi::new()
        .post("/api/v1/players/unknown/message", json!({"text": "hello"}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_broadcast_returns_200() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/players/broadcast",
            json!({"text": "hello everyone"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["success"], true);
    assert!(
        body["data"]["message"]
            .as_str()
            .unwrap()
            .contains("Broadcast")
    );
}

#[tokio::test]
async fn test_broadcast_text_too_long_returns_400() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/players/broadcast",
            json!({"text": "x".repeat(257)}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn test_message_text_too_long_returns_400() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/players/unknown/message",
            json!({"text": "x".repeat(257)}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn listing_players_by_an_unknown_mode_is_a_bad_request() {
    let (status, body) = TestApi::new().get("/api/v1/players?mode=turbo").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}
