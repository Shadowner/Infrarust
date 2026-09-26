use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use infrarust_plugin_admin_api::server_dir::test_support::{
    DocumentDuringSend, RecordingSender, SlowSender,
};
use infrarust_plugin_admin_api::server_dir::{parse_document, reload, watch};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::common::{TestApi, json_body, shadowed_config_service, text_body};

#[tokio::test]
async fn test_servers_list_returns_200() {
    let (status, body) = TestApi::new().get("/api/v1/servers").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"].is_array());
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_servers_get_returns_404_for_unknown() {
    let (status, body) = TestApi::new().get("/api/v1/servers/nonexistent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn test_servers_list_reports_source_and_editable() {
    let (status, body) = TestApi::new().get("/api/v1/servers").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"][0]["source"], "file");
    assert_eq!(body["data"][0]["editable"], false);
}

#[tokio::test]
async fn test_servers_raw_returns_toml() {
    let api = TestApi::new();
    let request = api
        .request(Method::GET, "/api/v1/servers/server_0/raw")
        .body(Body::empty())
        .unwrap();

    let response = api.send(request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/plain; charset=utf-8"
    );

    let text = text_body(response).await;
    assert!(text.contains("id = \"server_0\""));
}

#[tokio::test]
async fn test_servers_config_returns_the_whole_config_as_json() {
    let (status, body) = TestApi::new().get("/api/v1/servers/server_0/config").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["id"], "server_0");
    assert_eq!(body["data"]["addresses"][0], "127.0.0.1:25565");
    assert_eq!(body["data"]["proxy_mode"], "passthrough");
    assert_eq!(body["data"]["balance"], "first_available");
}

#[tokio::test]
async fn test_servers_config_unknown_is_not_found() {
    let (status, _) = TestApi::new().get("/api/v1/servers/nope/config").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_servers_write_to_file_provider_is_forbidden() {
    let (status, _) = TestApi::new()
        .put(
            "/api/v1/servers/server_0",
            json!({
                "id": "server_0",
                "domains": ["a.example.com"],
                "addresses": ["127.0.0.1:25565"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_servers_validate_accepts_valid_json() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/servers/validate",
            json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["127.0.0.1:25565"],
            }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["valid"], true);
    assert!(body["data"]["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_servers_validate_accepts_a_redacted_document() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/servers/validate",
            json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["127.0.0.1:25565"],
                "server_manager": {
                    "type": "pterodactyl",
                    "api_url": "https://panel.example.com",
                    "api_key": infrarust_config::secrets::REDACTED,
                    "server_id": "abc",
                },
            }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["valid"], true, "{body}");
}

#[tokio::test]
async fn test_servers_validate_reports_errors() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/servers/validate",
            json!({
                "id": "lobby",
                "domains": [],
                "addresses": ["127.0.0.1:25565"],
            }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["valid"], false);
    assert!(!body["data"]["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_servers_validate_reports_warnings() {
    let (status, body) = TestApi::new()
        .post(
            "/api/v1/servers/validate",
            json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["127.0.0.1:25565", "127.0.0.1:25566"],
            }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["valid"], true);
    assert!(!body["data"]["warnings"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_servers_create_persists_full_document() {
    let api = TestApi::new();

    let (status, _) = api
        .post(
            "/api/v1/servers",
            json!({
                "id": "survival",
                "domains": ["mc.example.com"],
                "addresses": [
                    "10.0.0.1:25565",
                    { "address": "10.0.0.2:25565", "weight": 3 },
                ],
                "balance": "least_conn",
                "slow_start": "45s",
                "motd": { "online": { "text": "Welcome" } },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let stored = std::fs::read_to_string(api.path().join("servers/survival.toml")).unwrap();
    let config = parse_document("survival", &stored).unwrap();
    assert_eq!(config.balance, infrarust_config::BalanceStrategy::LeastConn);
    assert_eq!(config.addresses[1].weight, 3);
    assert_eq!(config.motd.online.as_ref().unwrap().text, "Welcome");
    assert!(api.state.server_dir.owns("survival"));
}

#[tokio::test]
async fn test_servers_raw_hides_the_manager_api_key() {
    let api = TestApi::managed_server();

    let (status, _, text) = api
        .text(Method::GET, "/api/v1/servers/survival/raw", None)
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(!text.contains("ptlc_live_xxx"));
    assert!(text.contains(infrarust_config::secrets::REDACTED));
    assert!(text.contains("https://panel.example.com"));
}

#[tokio::test]
async fn test_servers_config_hides_the_manager_api_key() {
    let api = TestApi::managed_server();

    let (status, _, body) = api
        .text(Method::GET, "/api/v1/servers/survival/config", None)
        .await;
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"]["server_manager"]["api_key"],
        infrarust_config::secrets::REDACTED
    );
}

#[tokio::test]
async fn test_servers_raw_round_trip_keeps_the_manager_api_key() {
    let api = TestApi::managed_server();
    let (_, _, document) = api
        .text(Method::GET, "/api/v1/servers/survival/raw", None)
        .await;
    let edited = document.replace("mc.example.com", "play.example.com");

    let (status, _, _) = api
        .text(Method::PUT, "/api/v1/servers/survival/raw", Some(&edited))
        .await;

    assert_eq!(status, StatusCode::OK);
    let stored = std::fs::read_to_string(api.path().join("servers/survival.toml")).unwrap();
    assert!(stored.contains("ptlc_live_xxx"));
    assert!(stored.contains("play.example.com"));
    assert!(!stored.contains(infrarust_config::secrets::REDACTED));
}

#[tokio::test]
async fn test_servers_update_keeps_the_manager_api_key() {
    let api = TestApi::managed_server();
    let (_, _, body) = api
        .text(Method::GET, "/api/v1/servers/survival/config", None)
        .await;
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();

    let (status, _) = api
        .put("/api/v1/servers/survival", body["data"].clone())
        .await;

    assert_eq!(status, StatusCode::OK);
    let stored = std::fs::read_to_string(api.path().join("servers/survival.toml")).unwrap();
    assert!(stored.contains("ptlc_live_xxx"));
}

#[tokio::test]
async fn test_servers_create_rejects_a_secret_it_cannot_restore() {
    let api = TestApi::new();

    let (status, body) = api
        .post(
            "/api/v1/servers",
            json!({
                "id": "survival",
                "domains": ["mc.example.com"],
                "addresses": ["10.0.0.1:25565"],
                "server_manager": {
                    "type": "pterodactyl",
                    "api_url": "https://panel.example.com",
                    "api_key": infrarust_config::secrets::REDACTED,
                    "server_id": "abc",
                },
            }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "BAD_REQUEST");
    assert!(!api.path().join("servers/survival.toml").exists());
}

#[tokio::test]
async fn test_servers_create_refuses_a_taken_file_name() {
    let api = TestApi::builder()
        .server_file(
            "foo.toml",
            "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
        )
        .build();
    let path = api.path().join("servers/foo.toml");

    let (status, _) = api
        .post(
            "/api/v1/servers",
            json!({
                "id": "foo",
                "domains": ["foo.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(std::fs::read_to_string(&path).unwrap().contains("bar"));
}

#[tokio::test]
async fn test_servers_created_by_name_do_not_echo_back_as_an_update() {
    let api = TestApi::new();
    let sender = RecordingSender::default();
    *api.state.provider_sender.lock().await = Some(Box::new(sender.clone()));

    let (status, _) = api
        .post(
            "/api/v1/servers",
            json!({
                "name": "survival",
                "domains": ["mc.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let summary = reload(&api.state.server_dir, &api.state.provider_sender)
        .await
        .unwrap();
    assert_eq!((summary.added, summary.updated, summary.removed), (0, 0, 0));
    assert_eq!(sender.events(), vec!["added:survival".to_string()]);
}

#[tokio::test]
async fn test_a_write_reaches_the_proxy_before_the_directory_is_released() {
    let api = Arc::new(TestApi::new());
    let (sender, announcements) =
        DocumentDuringSend::new(api.state.server_dir.clone(), Duration::from_millis(100));
    *api.state.provider_sender.lock().await = Some(Box::new(sender));

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

    let writers: Vec<_> = (0..4)
        .map(|i| {
            let api = Arc::clone(&api);
            tokio::spawn(async move {
                api.put(
                    "/api/v1/servers/lobby",
                    json!({
                        "id": "lobby",
                        "domains": [format!("lobby{i}.example.com")],
                        "addresses": ["10.0.0.1:25565"],
                    }),
                )
                .await
                .0
            })
        })
        .collect();
    for writer in writers {
        assert_eq!(writer.await.unwrap(), StatusCode::OK);
    }

    let announcements = announcements.lock().unwrap().clone();
    assert_eq!(announcements.len(), 5);
    for (announced, stored) in announcements {
        assert_eq!(
            stored.as_deref(),
            Some(announced.as_str()),
            "the proxy was told about a document the directory no longer held"
        );
    }
}

#[tokio::test]
async fn test_writes_and_the_watcher_do_not_deadlock() {
    let api = Arc::new(TestApi::new());
    let sender = SlowSender::new(Duration::from_millis(100));
    *api.state.provider_sender.lock().await = Some(Box::new(sender));

    let shutdown = CancellationToken::new();
    tokio::spawn(watch(
        api.state.server_dir.clone(),
        api.state.provider_sender.clone(),
        shutdown.clone(),
    ));

    let statuses = tokio::time::timeout(Duration::from_secs(20), async {
        let writers: Vec<_> = (0..6)
            .map(|i| {
                let api = Arc::clone(&api);
                tokio::spawn(async move {
                    api.post(
                        "/api/v1/servers",
                        json!({
                            "id": format!("lobby-{i}"),
                            "domains": [format!("lobby{i}.example.com")],
                            "addresses": ["10.0.0.1:25565"],
                        }),
                    )
                    .await
                    .0
                })
            })
            .collect();

        let mut statuses = Vec::new();
        for writer in writers {
            statuses.push(writer.await.unwrap());
        }
        statuses
    })
    .await
    .expect("the writers and the directory watcher deadlocked");
    shutdown.cancel();

    assert_eq!(statuses.len(), 6);
    assert!(statuses.iter().all(|status| *status == StatusCode::CREATED));
}

#[tokio::test]
async fn test_servers_supplied_twice_are_not_editable() {
    let api = TestApi::builder()
        .server_file(
            "lobby.toml",
            "domains = [\"lobby.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n",
        )
        .config(Arc::new(shadowed_config_service("lobby")))
        .build();

    let (status, _) = api
        .put(
            "/api/v1/servers/lobby",
            json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["10.0.0.2:25565"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let request = api
        .request(Method::GET, "/api/v1/servers/lobby")
        .body(Body::empty())
        .unwrap();
    let detail = json_body(api.send(request).await).await;
    assert_eq!(detail["data"]["editable"], false);
}

#[tokio::test]
async fn test_servers_update_of_a_shadowed_document_is_rejected() {
    let api = TestApi::builder()
        .server_file(
            "foo.toml",
            "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
        )
        .build();

    let (status, _) = api
        .put(
            "/api/v1/servers/bar",
            json!({
                "name": "bar",
                "domains": ["bar.example.com"],
                "addresses": ["10.0.0.8:25565"],
            }),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!api.path().join("servers/bar.toml").exists());
}

#[tokio::test]
async fn test_servers_create_rejects_traversal_id() {
    let api = TestApi::new();

    let (status, _) = api
        .post(
            "/api/v1/servers",
            json!({
                "id": "../escaped",
                "domains": ["mc.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!api.path().join("servers/__escaped.toml").exists());
}

#[tokio::test]
async fn test_server_start_not_found() {
    let (status, body) = TestApi::new()
        .post("/api/v1/servers/nonexistent/start", json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn test_server_stop_not_found() {
    let (status, body) = TestApi::new()
        .post("/api/v1/servers/nonexistent/stop", json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}
