use std::sync::Arc;

use axum::http::{Method, StatusCode};
use infrarust_plugin_admin_api::server_dir::test_support::RecordingSender;
use serde_json::json;

use crate::common::{OVERRIDDEN_SERVERS_DIR, SECRET_API_KEY, TestApi, config_service};

#[tokio::test]
async fn test_config_providers_returns_200() {
    let (status, body) = TestApi::new().get("/api/v1/config/providers").await;
    assert_eq!(status, StatusCode::OK);
    let providers = body["data"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0]["provider_type"], "file");
    assert_eq!(providers[0]["configs_count"], 2);
}

#[tokio::test]
async fn test_config_reload_without_a_provider_is_unavailable() {
    let (status, body) = TestApi::new()
        .post("/api/v1/config/reload", json!({}))
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
}

#[tokio::test]
async fn test_config_reload_rescans_and_reports_what_it_found() {
    let api = TestApi::new();
    let sender = RecordingSender::default();
    *api.state.provider_sender.lock().await = Some(Box::new(sender.clone()));

    std::fs::write(
        api.path().join("servers/lobby.toml"),
        "domains = [\"lobby.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n",
    )
    .unwrap();

    let (status, body) = api.post("/api/v1/config/reload", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["details"]["added"], 1);
    assert_eq!(sender.events(), vec!["added:lobby".to_string()]);
}

#[tokio::test]
async fn test_config_proxy_json_fills_defaults_and_hides_the_api_key() {
    let (status, body) = TestApi::new().get("/api/v1/config/proxy").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["bind"], "0.0.0.0:25565");
    assert_eq!(body["data"]["web"]["bind"], "127.0.0.1:8080");
    assert_eq!(
        body["data"]["web"]["api_key"],
        infrarust_config::secrets::REDACTED
    );
    assert_eq!(body["data"]["connect_timeout"], "5s");
    assert!(!body.to_string().contains(SECRET_API_KEY));
}

#[tokio::test]
async fn test_config_proxy_json_reports_the_running_config() {
    let (status, body) = TestApi::new().get("/api/v1/config/proxy").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["servers_dir"], OVERRIDDEN_SERVERS_DIR);
}

#[tokio::test]
async fn test_config_proxy_raw_is_the_file_with_the_api_key_hidden() {
    let api = TestApi::new();
    let (status, content_type, text) = api
        .text(Method::GET, "/api/v1/config/proxy/raw", None)
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.unwrap(), "text/plain; charset=utf-8");
    assert!(!text.contains(SECRET_API_KEY));
    assert!(text.contains(infrarust_config::secrets::REDACTED));
    assert!(text.contains("./servers"), "the file's own value");
    assert!(text.contains("# the proxy listens here"));
}

#[tokio::test]
async fn test_config_proxy_raw_round_trip_keeps_the_api_key() {
    let config = Arc::new(config_service());
    let api = TestApi::with_config(Arc::clone(&config));
    let (_, _, document) = api
        .text(Method::GET, "/api/v1/config/proxy/raw", None)
        .await;

    let (status, _, body) = api
        .text(Method::PUT, "/api/v1/config/proxy/raw", Some(&document))
        .await;

    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["data"]["success"], true);
    assert_eq!(body["data"]["requires_restart"], true);

    let stored = config.stored_proxy_document();
    assert!(stored.contains("api_key = \"super-secret-key-value\""));
    assert!(stored.contains("# the proxy listens here"));
}

#[tokio::test]
async fn test_config_proxy_raw_rejects_a_malformed_document() {
    let api = TestApi::new();
    let before = api.config.stored_proxy_document();

    let (status, _, _) = api
        .text(
            Method::PUT,
            "/api/v1/config/proxy/raw",
            Some("bind = [unclosed"),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(api.config.stored_proxy_document(), before);
}

#[tokio::test]
async fn test_config_proxy_validate_accepts_a_valid_document() {
    let api = TestApi::new();
    let document = format!(
        "bind = \"0.0.0.0:25565\"\nservers_dir = {:?}\n",
        api.path().display()
    );

    let (status, _, body) = api
        .text(
            Method::POST,
            "/api/v1/config/proxy/validate",
            Some(&document),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["data"]["valid"], true);
}

#[tokio::test]
async fn test_config_proxy_validate_ignores_where_the_document_points() {
    let api = TestApi::new();
    let (_, _, document) = api
        .text(Method::GET, "/api/v1/config/proxy/raw", None)
        .await;
    assert!(document.contains("./servers"));

    let (status, _, body) = api
        .text(
            Method::POST,
            "/api/v1/config/proxy/validate",
            Some(&document),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["data"]["valid"], true, "{body}");
}

#[tokio::test]
async fn test_config_proxy_validate_reports_an_unknown_key() {
    let api = TestApi::new();

    let (status, _, body) = api
        .text(
            Method::POST,
            "/api/v1/config/proxy/validate",
            Some("bnid = \"0.0.0.0:25565\"\n"),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["data"]["valid"], false);
    assert!(!body["data"]["errors"].as_array().unwrap().is_empty());
}
