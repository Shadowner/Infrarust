use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use infrarust_api::BoxFuture;
use infrarust_api::services::{
    AvailableUpdate, InstallRequest, InstallState, ProxyUpdates, RestartRequirement,
    UpdateRequestError, UpdateStatus,
};
use serde_json::json;

use crate::common::TestApi;

struct MockUpdates {
    status: UpdateStatus,
    checked: Option<UpdateStatus>,
    install: Result<(), UpdateRequestError>,
    checks: AtomicUsize,
    installs: Mutex<Vec<InstallRequest>>,
}

impl MockUpdates {
    fn new(status: UpdateStatus) -> Self {
        Self {
            status,
            checked: None,
            install: Ok(()),
            checks: AtomicUsize::new(0),
            installs: Mutex::new(Vec::new()),
        }
    }

    fn checking_to(mut self, checked: UpdateStatus) -> Self {
        self.checked = Some(checked);
        self
    }

    fn installing(mut self, install: Result<(), UpdateRequestError>) -> Self {
        self.install = install;
        self
    }

    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }

    fn installs(&self) -> Vec<InstallRequest> {
        self.installs.lock().unwrap().clone()
    }
}

impl infrarust_api::__sealed::Sealed for MockUpdates {}

impl ProxyUpdates for MockUpdates {
    fn status(&self) -> UpdateStatus {
        self.status.clone()
    }

    fn check_now(&self) -> BoxFuture<'_, UpdateStatus> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match &self.checked {
                Some(checked) => checked.clone(),
                None => std::future::pending().await,
            }
        })
    }

    fn start_install(&self, request: InstallRequest) -> Result<(), UpdateRequestError> {
        self.installs.lock().unwrap().push(request);
        self.install.clone()
    }
}

fn up_to_date() -> UpdateStatus {
    UpdateStatus {
        current_version: "2.0.0-test".into(),
        checks_enabled: true,
        last_check: None,
        last_error: None,
        available: None,
        install: InstallState::Idle,
        install_refusal: None,
        binary_changed: false,
    }
}

fn security_update_installing() -> UpdateStatus {
    UpdateStatus {
        last_check: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_711_288_200)),
        last_error: Some("rate limited".into()),
        available: Some(AvailableUpdate {
            version: "2.1.0".into(),
            published_at: Some("2024-03-20T10:00:00Z".into()),
            notes_url: Some("https://example.com/releases/v2.1.0".into()),
            security: true,
            restart: RestartRequirement::Hot,
        }),
        install: InstallState::Running {
            version: "2.1.0".into(),
        },
        install_refusal: Some("built from source".into()),
        binary_changed: true,
        ..up_to_date()
    }
}

fn api_with(updates: &Arc<MockUpdates>) -> TestApi {
    TestApi::builder()
        .updates(Arc::clone(updates) as Arc<dyn ProxyUpdates>)
        .build()
}

fn install_request(api: &TestApi, body: Option<&str>) -> Request<Body> {
    let builder = api.request(Method::POST, "/api/v1/proxy/update/install");
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

#[tokio::test]
async fn update_status_without_a_service_reports_updates_disabled() {
    let (status, body) = TestApi::new().get("/api/v1/proxy/update").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"],
        json!({
            "enabled": false,
            "current_version": "2.0.0-test",
            "checks_enabled": false,
            "last_check": null,
            "last_error": null,
            "available": null,
            "install": { "state": "idle" },
            "install_refusal": null,
            "binary_changed": false,
        })
    );
}

#[tokio::test]
async fn update_status_serves_the_cached_status_without_checking() {
    let updates = Arc::new(MockUpdates::new(security_update_installing()));

    let (status, body) = api_with(&updates).get("/api/v1/proxy/update").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"],
        json!({
            "enabled": true,
            "current_version": "2.0.0-test",
            "checks_enabled": true,
            "last_check": "2024-03-24T13:50:00Z",
            "last_error": "rate limited",
            "available": {
                "version": "2.1.0",
                "published_at": "2024-03-20T10:00:00Z",
                "notes_url": "https://example.com/releases/v2.1.0",
                "security": true,
                "restart": "hot",
            },
            "install": { "state": "running", "version": "2.1.0" },
            "install_refusal": "built from source",
            "binary_changed": true,
        })
    );
    assert_eq!(updates.checks(), 0);
}

#[tokio::test]
async fn update_status_names_each_install_state() {
    let cases = [
        (InstallState::Idle, json!({ "state": "idle" })),
        (
            InstallState::Installed {
                version: "2.1.0".into(),
            },
            json!({ "state": "installed", "version": "2.1.0" }),
        ),
        (
            InstallState::Failed {
                reason: "checksum mismatch".into(),
            },
            json!({ "state": "failed", "reason": "checksum mismatch" }),
        ),
    ];
    for (install, expected) in cases {
        let updates = Arc::new(MockUpdates::new(UpdateStatus {
            install,
            ..up_to_date()
        }));

        let (_, body) = api_with(&updates).get("/api/v1/proxy/update").await;

        assert_eq!(body["data"]["install"], expected);
    }
}

#[tokio::test]
async fn update_check_returns_the_fresh_status() {
    let updates =
        Arc::new(MockUpdates::new(up_to_date()).checking_to(security_update_installing()));

    let (status, body) = api_with(&updates)
        .post("/api/v1/proxy/update/check", json!({}))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["enabled"], true);
    assert_eq!(body["data"]["available"]["version"], "2.1.0");
    assert_eq!(updates.checks(), 1);
}

#[tokio::test(start_paused = true)]
async fn update_check_that_outlives_its_budget_answers_before_the_request_timeout() {
    let updates = Arc::new(MockUpdates::new(up_to_date()));

    let (status, body) = api_with(&updates)
        .post("/api/v1/proxy/update/check", json!({}))
        .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("did not finish")
    );
    assert_eq!(updates.checks(), 1);
}

#[tokio::test]
async fn update_check_without_a_service_is_unavailable() {
    let (status, body) = TestApi::new()
        .post("/api/v1/proxy/update/check", json!({}))
        .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
}

#[tokio::test]
async fn update_install_is_accepted_without_restarting() {
    let updates = Arc::new(MockUpdates::new(security_update_installing()));
    let api = api_with(&updates);

    let (status, body) = api.json(install_request(&api, Some("{}"))).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["data"]["success"], true);
    assert_eq!(body["data"]["message"], "Installing Infrarust 2.1.0");
    assert_eq!(
        updates.installs(),
        vec![InstallRequest {
            version: None,
            then_restart: false,
        }]
    );
}

#[tokio::test]
async fn update_install_accepts_an_empty_request() {
    let updates = Arc::new(MockUpdates::new(up_to_date()));
    let api = api_with(&updates);

    let (status, body) = api.json(install_request(&api, None)).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["data"]["message"], "Installing the latest update");
    assert_eq!(updates.installs().len(), 1);
}

#[tokio::test]
async fn update_install_passes_the_requested_version() {
    let updates = Arc::new(MockUpdates::new(up_to_date()));
    let api = api_with(&updates);

    let (status, body) = api
        .json(install_request(&api, Some(r#"{"version":" 2.1.0 "}"#)))
        .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["data"]["message"], "Installing Infrarust 2.1.0");
    assert_eq!(
        updates.installs(),
        vec![InstallRequest {
            version: Some("2.1.0".into()),
            then_restart: false,
        }]
    );
}

#[tokio::test]
async fn update_install_rejects_a_blank_version() {
    let updates = Arc::new(MockUpdates::new(up_to_date()));
    let api = api_with(&updates);

    let (status, body) = api
        .json(install_request(&api, Some(r#"{"version":"  "}"#)))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
    assert!(updates.installs().is_empty());
}

#[tokio::test]
async fn update_install_maps_each_refusal_to_its_status() {
    let cases = [
        (
            UpdateRequestError::Busy,
            StatusCode::CONFLICT,
            "CONFLICT",
            "already running",
        ),
        (
            UpdateRequestError::NothingToInstall,
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "no update to install",
        ),
        (
            UpdateRequestError::Refused("run cargo install infrarust --version 2.1.0".into()),
            StatusCode::CONFLICT,
            "CONFLICT",
            "cargo install infrarust --version 2.1.0",
        ),
    ];
    for (error, expected_status, expected_code, expected_message) in cases {
        let updates = Arc::new(MockUpdates::new(up_to_date()).installing(Err(error)));
        let api = api_with(&updates);

        let (status, body) = api.json(install_request(&api, Some("{}"))).await;

        assert_eq!(status, expected_status, "{body}");
        assert_eq!(body["error"]["code"], expected_code);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains(expected_message),
            "{body}"
        );
    }
}

#[tokio::test]
async fn update_install_without_a_service_is_unavailable() {
    let api = TestApi::new();

    let (status, body) = api.json(install_request(&api, Some("{}"))).await;

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
async fn update_routes_require_authentication() {
    let updates = Arc::new(MockUpdates::new(up_to_date()).checking_to(up_to_date()));
    let api = api_with(&updates);

    for (method, uri) in [
        (Method::GET, "/api/v1/proxy/update"),
        (Method::POST, "/api/v1/proxy/update/check"),
        (Method::POST, "/api/v1/proxy/update/install"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();

        let (status, body) = api.json(request).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(body["error"]["code"], "UNAUTHORIZED");
    }
    assert_eq!(updates.checks(), 0);
    assert!(updates.installs().is_empty());
}
