use axum::body::Body;
use axum::http::{Request, StatusCode, header};

use crate::common::{TestApi, json_body, unauthenticated};

fn with_authorization(uri: &str, value: &'static str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::AUTHORIZATION, value)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn test_health_returns_200_without_auth() {
    let api = TestApi::new();
    let response = api.send(unauthenticated("/api/v1/health")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn test_proxy_status_returns_200_with_auth() {
    let (status, _) = TestApi::new().get("/api/v1/proxy").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_proxy_status_returns_401_without_auth_with_error_body() {
    let api = TestApi::new();
    let response = api.send(unauthenticated("/api/v1/proxy")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "UNAUTHORIZED");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing")
    );
}

#[tokio::test]
async fn test_proxy_status_returns_401_with_bad_key() {
    let api = TestApi::new();
    let response = api
        .send(with_authorization("/api/v1/proxy", "Bearer wrong-key"))
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "UNAUTHORIZED");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid")
    );
}

#[tokio::test]
async fn test_empty_configured_key_rejects_empty_token() {
    let api = TestApi::with_key("");
    let response = api
        .send(with_authorization("/api/v1/proxy", "Bearer "))
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_failed_auth_is_rate_limited() {
    let api = TestApi::with_rate_limit(2);

    for _ in 0..2 {
        let response = api
            .send(with_authorization("/api/v1/proxy", "Bearer wrong-key"))
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let response = api
        .send(with_authorization("/api/v1/proxy", "Bearer wrong-key"))
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_sse_routes_are_rate_limited() {
    let api = TestApi::with_rate_limit(1);

    let first = api.send(unauthenticated("/api/v1/events")).await;
    assert_eq!(first.status(), StatusCode::UNAUTHORIZED);

    let second = api.send(unauthenticated("/api/v1/events")).await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_proxy_status_returns_401_with_non_bearer() {
    let api = TestApi::new();
    let response = api
        .send(with_authorization("/api/v1/proxy", "Basic dXNlcjpwYXNz"))
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_proxy_status_contains_expected_fields() {
    let (status, body) = TestApi::new().get("/api/v1/proxy").await;
    assert_eq!(status, StatusCode::OK);

    let data = &body["data"];
    assert_eq!(data["version"], "2.0.0-test");
    assert_eq!(data["players_online"], 3);
    assert_eq!(data["servers_count"], 2);
    assert!(data["uptime_seconds"].is_u64());
    assert!(data["uptime_human"].is_string());
    assert!(data["bind_address"].is_string());
    assert!(data["features"].is_array());
}

#[tokio::test]
async fn test_unknown_route_returns_404() {
    let api = TestApi::new();
    let response = api.send(unauthenticated("/api/v1/unknown")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

const PROTECTED_ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/v1/proxy"),
    ("GET", "/api/v1/players"),
    ("GET", "/api/v1/players/count"),
    ("GET", "/api/v1/players/Steve"),
    ("GET", "/api/v1/bans"),
    ("POST", "/api/v1/bans"),
    ("GET", "/api/v1/bans/check/username/test"),
    ("GET", "/api/v1/servers"),
    ("POST", "/api/v1/servers"),
    ("POST", "/api/v1/servers/validate"),
    ("GET", "/api/v1/servers/server_0"),
    ("PUT", "/api/v1/servers/server_0"),
    ("DELETE", "/api/v1/servers/server_0"),
    ("GET", "/api/v1/servers/server_0/raw"),
    ("PUT", "/api/v1/servers/server_0/raw"),
    ("GET", "/api/v1/servers/server_0/config"),
    ("GET", "/api/v1/servers/server_0/backends"),
    ("GET", "/api/v1/health/backends"),
    ("GET", "/api/v1/plugins"),
    ("GET", "/api/v1/plugins/test"),
    ("GET", "/api/v1/stats"),
    ("GET", "/api/v1/events/recent"),
    ("GET", "/api/v1/config/providers"),
    ("GET", "/api/v1/config/proxy"),
    ("GET", "/api/v1/config/proxy/raw"),
    ("PUT", "/api/v1/config/proxy/raw"),
    ("POST", "/api/v1/config/proxy/validate"),
    ("GET", "/api/v1/logs/history"),
    ("POST", "/api/v1/players/broadcast"),
    ("POST", "/api/v1/players/test/kick"),
    ("POST", "/api/v1/players/test/send"),
    ("POST", "/api/v1/players/test/message"),
    ("DELETE", "/api/v1/bans/username/test"),
    ("POST", "/api/v1/servers/test/start"),
    ("POST", "/api/v1/servers/test/stop"),
    ("GET", "/api/v1/servers/test/health"),
    ("GET", "/api/v1/servers/test/health/cached"),
    (
        "POST",
        "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
    ),
    (
        "POST",
        "/api/v1/servers/server_0/backends/10.0.0.0:25565/enable",
    ),
    (
        "POST",
        "/api/v1/servers/server_0/backends/10.0.0.0:25565/reset",
    ),
    ("POST", "/api/v1/config/reload"),
    ("POST", "/api/v1/proxy/shutdown"),
];

#[tokio::test]
async fn every_protected_route_requires_auth() {
    for &(method, uri) in PROTECTED_ROUTES {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let response = TestApi::new().send(request).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "Expected 401 for {method} {uri}"
        );
    }
}

#[test]
fn the_protected_route_list_covers_every_handler_of_the_router() {
    let router = include_str!("../../src/router.rs");
    let start = router.find("let protected_routes").unwrap();
    let end = router.find("let sse_routes").unwrap();
    let section = &router[start..end];

    let handlers: usize = ["get(", "post(", "put(", "delete("]
        .iter()
        .flat_map(|method| ["handlers::", "sse::"].map(|module| format!("{method}{module}")))
        .map(|registration| section.matches(&registration).count())
        .sum();

    assert_eq!(PROTECTED_ROUTES.len(), handlers);
}

#[tokio::test]
async fn test_config_proxy_requires_auth() {
    let api = TestApi::new();
    let response = api.send(unauthenticated("/api/v1/config/proxy/raw")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
