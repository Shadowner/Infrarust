#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::proxy::ProxyPingEvent;
use infrarust_api::types::Component;
use infrarust_test_harness::legacy::LEGACY_PROTOCOL;
use infrarust_test_harness::text::json_text;
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeLegacyBackend, LegacyClient, LegacyPing,
    ProtocolVersion, Recorder, ScriptedPlugin, ServerSpec, TestProxy, TestProxyBuilder,
    version_matrix,
};
use serde_json::json;
use toml::{Table, Value};
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;
const DEFAULT_MOTD: &str = "Nothing lives here";

fn with_default_motd(builder: TestProxyBuilder) -> TestProxyBuilder {
    builder.patch_config(|table| {
        let online = Table::from_iter([
            ("text".to_string(), Value::String(DEFAULT_MOTD.into())),
            ("max_players".to_string(), Value::Integer(42)),
        ]);
        table.insert(
            "default_motd".into(),
            Value::Table(Table::from_iter([(
                "online".to_string(),
                Value::Table(online),
            )])),
        );
    })
}

async fn an_unknown_domain_is_answered_with_the_default_motd(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = with_default_motd(
        TestProxy::builder()
            .server(ServerSpec::offline("lobby").backend(backend.addr()))
            .plugin(recorder.plugin()),
    )
    .start()
    .await
    .unwrap();

    let status = proxy
        .client(version)
        .domain("nowhere.test")
        .status()
        .await
        .unwrap();

    assert_eq!(json_text(&status.json["description"]), DEFAULT_MOTD);
    assert_eq!(status.json["players"]["max"], json!(42));
    let pings = recorder.of(EventKind::ProxyPing);
    assert_eq!(pings.len(), 1, "{pings:?}");
    assert_eq!(pings[0].detail["description"], json!(DEFAULT_MOTD));
    assert_eq!(pings[0].detail["server"], json!(null));
    assert_eq!(pings[0].detail["virtual_host"], json!("nowhere.test"));
    assert_eq!(pings[0].detail["protocol_version"], json!(version.0));
    assert_eq!(pings[0].detail["legacy"], json!(false));
    assert_eq!(backend.status_requests(), 0);

    proxy.shutdown().await.unwrap();
}

version_matrix!(an_unknown_domain_is_answered_with_the_default_motd; p47 = 47, p764 = 764, p774 = 774);

fn legacy_backend_ping() -> LegacyPing {
    LegacyPing {
        protocol: Some(i32::from(LEGACY_PROTOCOL)),
        version: Some("1.6.4".into()),
        motd: "\u{a7}6A legacy backend".into(),
        online: 3,
        max: 20,
    }
}

fn motd_rewriter() -> ScriptedPlugin {
    ScriptedPlugin::new("motd").on::<ProxyPingEvent>(EventPriority::NORMAL, |event| {
        event.response.description = Component::text("Plugin MOTD");
        event.response.online_players = 7;
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_ping_carries_the_plugin_motd() {
    let backend = FakeLegacyBackend::spawn(legacy_backend_ping())
        .await
        .unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").backend(backend.addr()))
        .plugin(motd_rewriter())
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let ping = proxy
        .legacy_client_for("old")
        .unwrap()
        .ping()
        .await
        .unwrap();

    assert_eq!(ping.motd, "Plugin MOTD");
    assert_eq!(ping.online, 7);
    assert_eq!(ping.max, 20);
    assert_eq!(ping.protocol, Some(i32::from(LEGACY_PROTOCOL)));
    assert_eq!(ping.version.as_deref(), Some("1.6.4"));
    let pings = recorder.of(EventKind::ProxyPing);
    assert_eq!(pings.len(), 1, "{pings:?}");
    let ping = &pings[0].detail;
    assert_eq!(ping["legacy"], json!(true));
    assert_eq!(ping["server"], json!("old"));
    assert_eq!(ping["virtual_host"], json!("old.test"));
    assert_eq!(ping["protocol_version"], json!(i32::from(LEGACY_PROTOCOL)));
    assert_eq!(ping["description"], json!("Plugin MOTD"));
    assert_eq!(ping["max_players"], json!(20));

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untouched_legacy_ping_keeps_the_backend_motd() {
    let backend = FakeLegacyBackend::spawn(legacy_backend_ping())
        .await
        .unwrap();
    let counts =
        ScriptedPlugin::new("counts").on::<ProxyPingEvent>(EventPriority::NORMAL, |event| {
            event.response.max_players = 99;
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("old").backend(backend.addr()))
        .plugin(counts)
        .start()
        .await
        .unwrap();

    let ping = proxy
        .legacy_client_for("old")
        .unwrap()
        .ping()
        .await
        .unwrap();

    assert_eq!(
        ping,
        LegacyPing {
            max: 99,
            ..legacy_backend_ping()
        }
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_beta_ping_carries_the_plugin_motd() {
    let backend = FakeLegacyBackend::spawn(legacy_backend_ping())
        .await
        .unwrap();
    let proxy = with_default_motd(
        TestProxy::builder()
            .server(ServerSpec::passthrough("old").backend(backend.addr()))
            .plugin(motd_rewriter()),
    )
    .start()
    .await
    .unwrap();

    let ping = LegacyClient::new(proxy.addr()).ping_beta().await.unwrap();

    assert_eq!(ping.motd, "Plugin MOTD");
    assert_eq!(ping.online, 7);
    assert_eq!(ping.max, 42);

    proxy.shutdown().await.unwrap();
}

async fn a_timed_out_ping_listener_still_gets_an_answer(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let stuck = ScriptedPlugin::new("stuck")
        .on_async::<ProxyPingEvent>(EventPriority::NORMAL, |_| Box::pin(std::future::pending()));
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(stuck)
        .patch_config(|table| {
            table.insert(
                "events".into(),
                Value::Table(Table::from_iter([(
                    "handler_timeout".to_string(),
                    Value::String("200ms".into()),
                )])),
            );
        })
        .start()
        .await
        .unwrap();

    let status = proxy.client(version).status().await.unwrap();
    assert_eq!(
        json_text(&status.json["description"]),
        "Infrarust fake backend"
    );

    let legacy = LegacyClient::new(proxy.addr()).ping_beta().await.unwrap();
    assert_eq!(legacy.motd, "An Infrarust Proxy");

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_timed_out_ping_listener_still_gets_an_answer; p47 = 47, p764 = 764, p774 = 774);

async fn ping_fields_describe_the_request(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    proxy
        .client(version)
        .domain("Lobby.Test\0FML2\0")
        .status()
        .await
        .unwrap();

    let ping = recorder
        .wait_for_kind(EventKind::ProxyPing, T)
        .await
        .unwrap();
    assert_eq!(ping.detail["server"], json!("lobby"));
    assert_eq!(ping.detail["virtual_host"], json!("lobby.test"));
    assert_eq!(ping.detail["protocol_version"], json!(version.0));
    assert_eq!(ping.detail["legacy"], json!(false));
    assert_eq!(ping.detail["description"], json!("Infrarust fake backend"));
    let remote = ping.detail["remote_addr"].as_str().unwrap();
    assert!(remote.starts_with("127.0.0.1:"), "{remote}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(ping_fields_describe_the_request; p47 = 47, p764 = 764, p774 = 774);

const NOTCH: Uuid = Uuid::from_u128(0x069a_79f4_44e9_4726_a5be_fca9_0e38_aaf5);
const HEROBRINE: Uuid = Uuid::from_u128(0x00ff_00ff_00ff_00ff_00ff_00ff_00ff_00ff);

async fn the_player_sample_can_be_read_and_rewritten(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .status(json!({
            "version": { "name": "Harness", "protocol": version.0 },
            "players": {
                "max": 20,
                "online": 1,
                "sample": [{ "name": "Notch", "id": NOTCH.to_string() }],
            },
            "description": { "text": "Sampled" },
        }))
        .spawn()
        .await
        .unwrap();
    let seen: Arc<Mutex<Vec<(String, Uuid)>>> = Arc::default();
    let observed = Arc::clone(&seen);
    let sampler =
        ScriptedPlugin::new("sampler").on::<ProxyPingEvent>(EventPriority::NORMAL, move |event| {
            observed
                .lock()
                .unwrap()
                .clone_from(&event.response.player_sample);
            event
                .response
                .player_sample
                .push(("Herobrine".into(), HEROBRINE));
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(sampler)
        .start()
        .await
        .unwrap();

    let status = proxy.client(version).status().await.unwrap();

    assert_eq!(*seen.lock().unwrap(), [("Notch".to_string(), NOTCH)]);
    assert_eq!(
        status.json["players"]["sample"],
        json!([
            { "name": "Notch", "id": NOTCH.to_string() },
            { "name": "Herobrine", "id": HEROBRINE.to_string() },
        ])
    );
    assert_eq!(json_text(&status.json["description"]), "Sampled");

    proxy.shutdown().await.unwrap();
}

version_matrix!(the_player_sample_can_be_read_and_rewritten; p47 = 47, p764 = 764, p774 = 774);

async fn an_untouched_player_sample_is_relayed_verbatim(version: ProtocolVersion) {
    let sample = json!([
        { "name": "\u{a7}6Welcome", "id": "00000000-0000-0000-0000-000000000000" },
        { "name": "odd", "id": "not-a-uuid" },
    ]);
    let backend = FakeBackend::builder()
        .status(json!({
            "version": { "name": "Harness", "protocol": version.0 },
            "players": { "max": 20, "online": 2, "sample": sample.clone() },
            "description": { "text": "Sampled" },
        }))
        .spawn()
        .await
        .unwrap();
    let counts =
        ScriptedPlugin::new("counts").on::<ProxyPingEvent>(EventPriority::NORMAL, |event| {
            event.response.online_players = 5;
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(counts)
        .start()
        .await
        .unwrap();

    let status = proxy.client(version).status().await.unwrap();

    assert_eq!(status.json["players"]["sample"], sample);
    assert_eq!(status.json["players"]["online"], json!(5));

    proxy.shutdown().await.unwrap();
}

version_matrix!(an_untouched_player_sample_is_relayed_verbatim; p47 = 47, p764 = 764, p774 = 774);
