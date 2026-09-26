use axum::http::{Method, StatusCode};
use infrarust_plugin_admin_api::drain_store::DrainStore;
use infrarust_plugin_admin_api::util::parse_address;
use serde_json::json;

use crate::common::TestApi;

#[tokio::test]
async fn test_backends_list_reports_strategy_and_addresses() {
    let (status, body) = TestApi::new()
        .get("/api/v1/servers/server_0/backends")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["strategy"], "least_conn");

    let backend = &body["data"]["backends"][0];
    assert_eq!(backend["address"], "10.0.0.0:25565");
    assert_eq!(backend["state"], "healthy");
    assert_eq!(backend["weight"], 2);
    assert_eq!(backend["effective_weight"], 1);
    assert_eq!(backend["active_connections"], 4);
    assert_eq!(backend["ejections"], 1);
    assert_eq!(backend["healthy_since_secs"], 30);
    assert_eq!(backend["last_failure_secs_ago"], 90);
}

#[tokio::test]
async fn test_backends_list_unknown_server_returns_404() {
    let (status, _) = TestApi::new().get("/api/v1/servers/ghost/backends").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_proxy_wide_backends_are_grouped_by_server() {
    let (status, body) = TestApi::new().get("/api/v1/health/backends").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"]["server_0"]["backends"][0]["address"],
        "10.0.0.0:25565"
    );
    assert_eq!(body["data"]["server_1"]["strategy"], "least_conn");
}

#[tokio::test]
async fn test_drain_then_enable_round_trip() {
    let api = TestApi::new();

    let (status, _) = api
        .post(
            "/api/v1/servers/server_0/backends/10.0.0.0%3A25565/drain",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = api.get("/api/v1/servers/server_0/backends").await;
    assert_eq!(body["data"]["backends"][0]["state"], "draining");

    let (status, _) = api
        .post(
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/enable",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = api.get("/api/v1/servers/server_0/backends").await;
    assert_eq!(body["data"]["backends"][0]["state"], "healthy");
}

#[tokio::test]
async fn test_drain_survives_the_next_start() {
    let api = TestApi::new();

    api.post(
        "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
        json!({}),
    )
    .await;

    assert_eq!(
        DrainStore::open(api.path()).entries(),
        vec![(
            "server_0".to_string(),
            parse_address("10.0.0.0:25565").unwrap()
        )]
    );
}

#[tokio::test]
async fn test_deleting_a_server_forgets_its_drained_backends() {
    let api = TestApi::new();

    let (status, _) = api
        .post(
            "/api/v1/servers",
            json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    api.state
        .drain_store
        .set("lobby", &parse_address("10.0.0.1:25565").unwrap(), true)
        .await
        .unwrap();

    let (status, _) = api.delete("/api/v1/servers/lobby").await;
    assert_eq!(status, StatusCode::OK);

    assert!(DrainStore::open(api.path()).entries().is_empty());
}

#[tokio::test]
async fn test_enabling_a_backend_clears_it_whatever_the_spelling() {
    let api = TestApi::new();

    let (status, _) = api
        .post(
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = api
        .post(
            "/api/v1/servers/server_0/backends/10.0.0.0:25565%20/enable",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        DrainStore::open(api.path()).entries().is_empty(),
        "a stale entry would re-drain the backend on the next start"
    );
}

#[tokio::test]
async fn test_reset_backend_leaves_the_drain_untouched() {
    let api = TestApi::new();

    let (status, _) = api
        .post(
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = api
        .post(
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/reset",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["success"], true);

    let (_, body) = api.get("/api/v1/servers/server_0/backends").await;
    assert_eq!(body["data"]["backends"][0]["state"], "draining");
}

#[tokio::test]
async fn test_backend_mutation_rejects_unknown_address() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/servers/server_0/backends/10.9.9.9:25565/drain",
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn test_backend_mutation_rejects_a_malformed_address() {
    let api = TestApi::new();
    let request = api
        .request(
            Method::POST,
            "/api/v1/servers/server_0/backends/not-an-address/drain",
        )
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, _) = api.json(request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
