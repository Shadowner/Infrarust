use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use infrarust_api::services::{
    ProxyRestart, RestartCancelError, RestartOptions, RestartRequestError, ScheduledRestart,
};
use serde_json::json;

use crate::common::TestApi;

const RESTART: &str = "/api/v1/proxy/restart";

struct MockRestart {
    outcome: Result<(), RestartRequestError>,
    cancel: Result<(), RestartCancelError>,
    scheduled: Option<SystemTime>,
    requests: AtomicUsize,
    asked: Mutex<Vec<RestartOptions>>,
}

impl MockRestart {
    fn answering(outcome: Result<(), RestartRequestError>) -> Arc<Self> {
        Arc::new(Self::with(outcome))
    }

    fn with(outcome: Result<(), RestartRequestError>) -> Self {
        Self {
            outcome,
            cancel: Err(RestartCancelError::NothingScheduled),
            scheduled: None,
            requests: AtomicUsize::new(0),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn asked(&self) -> Vec<RestartOptions> {
        self.asked.lock().unwrap().clone()
    }
}

impl infrarust_api::__sealed::Sealed for MockRestart {}

impl ProxyRestart for MockRestart {
    fn request_restart_with(&self, options: RestartOptions) -> Result<(), RestartRequestError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.asked.lock().unwrap().push(options);
        self.outcome.clone()
    }

    fn cancel_restart(&self) -> Result<(), RestartCancelError> {
        self.cancel.clone()
    }

    fn scheduled_restart(&self) -> Option<ScheduledRestart> {
        self.scheduled.map(|at| ScheduledRestart { at })
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
    assert_eq!(restart.asked(), [RestartOptions::default()]);
}

#[tokio::test]
async fn restart_takes_a_delay_and_a_message() {
    let restart = MockRestart::answering(Ok(()));

    let (status, body) = api_with(&restart)
        .post(
            RESTART,
            json!({ "delay_seconds": 30, "message": "&eUpdate in {delay}" }),
        )
        .await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        body["data"]["message"],
        "Hot restart scheduled in 30 seconds"
    );
    assert_eq!(
        restart.asked(),
        [RestartOptions {
            delay: Some(Duration::from_secs(30)),
            message: Some("&eUpdate in {delay}".into()),
        }]
    );
}

#[tokio::test]
async fn restart_with_no_delay_overrides_the_configured_one() {
    let restart = MockRestart::answering(Ok(()));

    let (status, body) = api_with(&restart)
        .post(RESTART, json!({ "delay_seconds": 0 }))
        .await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["data"]["message"], "Hot restart scheduled");
    assert_eq!(
        restart.asked(),
        [RestartOptions {
            delay: Some(Duration::ZERO),
            message: None,
        }]
    );
}

#[tokio::test]
async fn restart_refuses_unknown_fields_and_options_over_the_limits() {
    let restart = MockRestart::answering(Ok(()));
    let api = api_with(&restart);
    let request = api
        .request(Method::POST, RESTART)
        .header("content-type", "application/json")
        .body(Body::from(r#"{"delay":"30s"}"#))
        .unwrap();
    assert!(api.send(request).await.status().is_client_error());
    assert_eq!(restart.requests(), 0);

    let refusing = MockRestart::answering(Err(RestartRequestError::Invalid(
        "the delay is 2h, the longest delay is 1h".into(),
    )));
    let (status, body) = api_with(&refusing)
        .post(RESTART, json!({ "delay_seconds": 7200 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("the longest delay is 1h"),
        "{body}"
    );
}

#[tokio::test]
async fn a_scheduled_restart_can_be_cancelled() {
    let restart = Arc::new(MockRestart {
        cancel: Ok(()),
        ..MockRestart::with(Ok(()))
    });

    let (status, body) = api_with(&restart).delete(RESTART).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"],
        json!({ "success": true, "message": "Hot restart cancelled" })
    );
    assert_eq!(restart.requests(), 0);
}

#[tokio::test]
async fn cancelling_when_nothing_can_be_cancelled_is_a_conflict() {
    for (error, expected) in [
        (
            RestartCancelError::NothingScheduled,
            "no restart is scheduled",
        ),
        (RestartCancelError::TooLate, "already under way"),
    ] {
        let restart = Arc::new(MockRestart {
            cancel: Err(error),
            ..MockRestart::with(Ok(()))
        });

        let (status, body) = api_with(&restart).delete(RESTART).await;

        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains(expected),
            "{body}"
        );
    }
    let (status, _) = TestApi::new().delete(RESTART).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn proxy_status_shows_when_a_restart_is_scheduled() {
    let idle = MockRestart::answering(Ok(()));
    let (_, body) = api_with(&idle).get("/api/v1/proxy").await;
    assert!(body["data"].get("restart_scheduled_at").is_none(), "{body}");
    assert!(body["data"].get("restart_in_seconds").is_none(), "{body}");

    let scheduled = Arc::new(MockRestart {
        scheduled: Some(SystemTime::now() + Duration::from_secs(90)),
        ..MockRestart::with(Ok(()))
    });
    let (_, body) = api_with(&scheduled).get("/api/v1/proxy").await;
    let left = body["data"]["restart_in_seconds"].as_u64().unwrap();
    assert!((85..=90).contains(&left), "{body}");
    let at = body["data"]["restart_scheduled_at"].as_str().unwrap();
    assert!(at.contains('T') && at.len() >= 20, "{at}");

    let overdue = Arc::new(MockRestart {
        scheduled: Some(SystemTime::now() - Duration::from_secs(5)),
        ..MockRestart::with(Ok(()))
    });
    let (_, body) = api_with(&overdue).get("/api/v1/proxy").await;
    assert_eq!(body["data"]["restart_in_seconds"], 0, "{body}");
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
