use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use infrarust_api::services::{ProxyRestart, RestartRequestError};
use serde_json::json;

use crate::common::TestApi;

const RESTART: &str = "/api/v1/proxy/restart";

struct MockRestart {
    outcome: Result<(), RestartRequestError>,
    requests: AtomicUsize,
}

impl MockRestart {
    fn answering(outcome: Result<(), RestartRequestError>) -> Arc<Self> {
        Arc::new(Self {
            outcome,
            requests: AtomicUsize::new(0),
        })
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl infrarust_api::__sealed::Sealed for MockRestart {}

impl ProxyRestart for MockRestart {
    fn request_restart(&self) -> Result<(), RestartRequestError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.outcome.clone()
    }
}

fn api_with(restart: &Arc<MockRestart>) -> TestApi {
    TestApi::builder()
        .restart(Arc::clone(restart) as Arc<dyn ProxyRestart>)
        .build()
}

#[tokio::test]
async fn restart_is_accepted_once_scheduled() {
    let restart = MockRestart::answering(Ok(()));

    let (status, body) = api_with(&restart).post(RESTART, json!({})).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        body["data"],
        json!({ "success": true, "message": "Hot restart scheduled" })
    );
    assert_eq!(restart.requests(), 1);
}

#[tokio::test]
async fn restart_accepts_an_empty_request() {
    let restart = MockRestart::answering(Ok(()));
    let api = api_with(&restart);
    let request = api
        .request(Method::POST, RESTART)
        .body(Body::empty())
        .unwrap();

    let (status, _) = api.json(request).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(restart.requests(), 1);
}

#[tokio::test]
async fn restart_while_busy_is_a_conflict() {
    let cases = [
        (RestartRequestError::InProgress, "already in progress"),
        (RestartRequestError::ShuttingDown, "shutting down"),
    ];
    for (error, expected_message) in cases {
        let restart = MockRestart::answering(Err(error));

        let (status, body) = api_with(&restart).post(RESTART, json!({})).await;

        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "CONFLICT");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains(expected_message),
            "{body}"
        );
        assert_eq!(restart.requests(), 1);
    }
}

#[tokio::test]
async fn restart_on_a_platform_without_hot_restart_is_a_conflict_naming_the_reason() {
    let restart = MockRestart::answering(Err(RestartRequestError::Unsupported(
        "windows, restart the proxy by hand".into(),
    )));

    let (status, body) = api_with(&restart).post(RESTART, json!({})).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "CONFLICT");
    assert_eq!(
        body["error"]["message"],
        "Conflict: hot restart is not supported here: windows, restart the proxy by hand"
    );
    assert_eq!(restart.requests(), 1);
}

#[tokio::test]
async fn restart_without_a_service_is_unavailable() {
    let (status, body) = TestApi::new().post(RESTART, json!({})).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not available")
    );
}

#[tokio::test]
async fn restart_requires_authentication() {
    let restart = MockRestart::answering(Ok(()));
    let request = Request::builder()
        .method(Method::POST)
        .uri(RESTART)
        .body(Body::empty())
        .unwrap();

    let (status, body) = api_with(&restart).json(request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "UNAUTHORIZED");
    assert_eq!(restart.requests(), 0);
}

#[tokio::test]
async fn proxy_status_says_whether_hot_restart_is_available() {
    let restart = MockRestart::answering(Ok(()));

    let (_, without) = TestApi::new().get("/api/v1/proxy").await;
    let (_, with) = api_with(&restart).get("/api/v1/proxy").await;

    assert_eq!(without["data"]["restart_available"], false);
    assert_eq!(with["data"]["restart_available"], true);
    assert_eq!(restart.requests(), 0);
}
