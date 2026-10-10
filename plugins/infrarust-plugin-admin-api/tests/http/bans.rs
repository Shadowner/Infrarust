use std::sync::Arc;

use axum::http::StatusCode;
use infrarust_api::services::ban_service::{BanEntry, BanSource, BanTarget};
use infrarust_api::test_util::MockBanService;
use serde_json::json;

use crate::common::TestApi;

#[tokio::test]
async fn test_bans_list_returns_200_with_empty_list() {
    let (status, body) = TestApi::new().get("/api/v1/bans").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"].is_array());
    assert_eq!(body["meta"]["total"], 0);
}

#[tokio::test]
async fn test_bans_check_returns_not_banned() {
    let (status, body) = TestApi::new()
        .get("/api/v1/bans/check/username/Steve")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["banned"], false);
    assert!(body["data"]["ban"].is_null());
}

#[tokio::test]
async fn test_bans_check_invalid_target_type() {
    let (status, body) = TestApi::new()
        .get("/api/v1/bans/check/email/test@test.com")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn test_create_ban_returns_201() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/bans",
            json!({
                "target": {"type": "username", "value": "griefer"},
                "reason": "griefing"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["data"]["success"], true);
}

#[tokio::test]
async fn test_create_ban_invalid_ip() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/bans",
            json!({
                "target": {"type": "ip", "value": "not-an-ip"}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn test_delete_ban_not_found() {
    let (status, body) = TestApi::new().delete("/api/v1/bans/username/nobody").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn listing_bans_by_an_unknown_target_type_is_a_bad_request() {
    let (status, body) = TestApi::new().get("/api/v1/bans?target_type=email").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn bans_are_paged_by_the_query_string() {
    let mut bans = MockBanService::new();
    for n in 1..=7 {
        bans = bans.with_entry(BanEntry::new(
            format!("ban-{n}"),
            BanTarget::Username(format!("Griefer{n}")),
            BanSource::System,
        ));
    }
    let api = TestApi::builder().ban_service(Arc::new(bans)).build();

    let (status, body) = api.get("/api/v1/bans?page=2&per_page=5").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
    assert_eq!(body["meta"]["total"], 7);
    assert_eq!(body["meta"]["page"], 2);
    assert_eq!(body["meta"]["per_page"], 5);
    assert_eq!(body["meta"]["total_pages"], 2);
}
