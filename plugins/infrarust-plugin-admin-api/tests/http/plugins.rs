use axum::http::StatusCode;

use crate::common::TestApi;

#[tokio::test]
async fn test_plugins_list_returns_200() {
    let (status, body) = TestApi::new().get("/api/v1/plugins").await;
    assert_eq!(status, StatusCode::OK);
    let plugins = body["data"].as_array().unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0]["id"], "admin_api");
    assert_eq!(plugins[0]["state"], "enabled");
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
