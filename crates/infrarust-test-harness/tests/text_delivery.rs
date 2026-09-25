#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::event::EventPriority;
use infrarust_api::events::connection::ServerPreConnectEvent;
use infrarust_api::events::lifecycle::PreLoginEvent;
use infrarust_api::events::proxy::ProxyPingEvent;
use infrarust_api::types::{ClickEvent, Component, HoverEvent, NamedColor, ServerId};
use infrarust_test_harness::text::{component_json, json_text};
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin, ServerSpec,
    TestProxy, version_matrix,
};
use serde_json::{Value, json};

const T: Duration = DEFAULT_TIMEOUT;
const DOCS_URL: &str = "https://example.com/docs";

fn styled_denial(text: &str) -> Component {
    Component::text(text).color(NamedColor::Red).bold()
}

fn styled_denial_json(text: &str) -> Value {
    json!({ "text": text, "color": "red", "bold": true })
}

fn snake_case_events(version: ProtocolVersion) -> bool {
    version.no_less_than(ProtocolVersion::V1_21_5)
}

fn event_keys(version: ProtocolVersion) -> (&'static str, &'static str) {
    if snake_case_events(version) {
        ("click_event", "hover_event")
    } else {
        ("clickEvent", "hoverEvent")
    }
}

fn open_url(version: ProtocolVersion) -> Value {
    if snake_case_events(version) {
        json!({ "action": "open_url", "url": DOCS_URL })
    } else {
        json!({ "action": "open_url", "value": DOCS_URL })
    }
}

fn tooltip_text(hover: &Value) -> String {
    let tooltip = hover
        .get("value")
        .or_else(|| hover.get("contents"))
        .unwrap_or_else(|| panic!("show_text hover without a tooltip: {hover}"));
    json_text(tooltip)
}

fn deny_server(server: &'static str, reason: &'static str) -> ScriptedPlugin {
    ScriptedPlugin::new("gate").on::<ServerPreConnectEvent>(EventPriority::NORMAL, move |event| {
        if event.server == ServerId::new(server) {
            event.deny(styled_denial(reason));
        }
    })
}

async fn pre_login_denial_keeps_text_and_style(version: ProtocolVersion) {
    let gate = ScriptedPlugin::new("gate").on::<PreLoginEvent>(EventPriority::NORMAL, |event| {
        if event.profile.username == "Mallory" {
            event.deny(styled_denial("No Mallory"));
        }
    });
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(gate)
        .start()
        .await
        .unwrap();

    let denied = proxy
        .client(version)
        .login("Mallory")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, "No Mallory", "{denied:?}");
    assert_eq!(denied.json, Some(styled_denial_json("No Mallory")));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, pre_login_denial_keeps_text_and_style);

async fn server_pre_connect_denial_is_exact(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(deny_server("lobby", "Lobby is closed"))
        .start()
        .await
        .unwrap();

    let denied = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, "Lobby is closed", "{denied:?}");
    assert_eq!(denied.json, Some(styled_denial_json("Lobby is closed")));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, server_pre_connect_denial_is_exact);

async fn passthrough_pre_connect_denial_is_exact(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(backend.addr()))
        .plugin(deny_server("pipe", "Pipe is closed"))
        .start()
        .await
        .unwrap();

    let denied = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, "Pipe is closed", "{denied:?}");
    assert_eq!(denied.json, Some(styled_denial_json("Pipe is closed")));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, passthrough_pre_connect_denial_is_exact);

fn napping(spec: ServerSpec, message: &'static str) -> ServerSpec {
    spec.unreachable().patch(move |table| {
        table.insert(
            "disconnect_message".into(),
            toml::Value::String(message.into()),
        );
    })
}

async fn unreachable_backend_shows_the_configured_message(version: ProtocolVersion) {
    let proxy = TestProxy::builder()
        .server(napping(ServerSpec::offline("lobby"), "Lobby is napping"))
        .server(napping(ServerSpec::passthrough("pipe"), "Pipe is napping"))
        .start()
        .await
        .unwrap();

    for (server, message) in [("lobby", "Lobby is napping"), ("pipe", "Pipe is napping")] {
        let denied = proxy
            .client_for(server, version)
            .unwrap()
            .login("Steve")
            .await
            .unwrap()
            .disconnected()
            .unwrap();
        assert_eq!(denied.state, ConnectionState::Login, "{server}");
        assert_eq!(denied.text, message, "{server}: {denied:?}");
        let plain = [json!(message), json!({ "text": message })];
        assert!(
            denied
                .json
                .as_ref()
                .is_some_and(|json| plain.contains(json)),
            "{server}: {denied:?}"
        );
    }

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, unreachable_backend_shows_the_configured_message);

async fn plugin_message_events_follow_the_client_version(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    player
        .send_message(
            Component::text("Docs")
                .click(ClickEvent::OpenUrl(DOCS_URL.into()))
                .hover(HoverEvent::show_text("Open the docs")),
        )
        .unwrap();
    let raw = session.expect_system_message(T).await.unwrap();
    let message = component_json(&raw, version).unwrap();

    let (click, hover) = event_keys(version);
    let (stale_click, stale_hover) = event_keys(if snake_case_events(version) {
        ProtocolVersion::V1_21_4
    } else {
        ProtocolVersion::V1_21_5
    });
    assert_eq!(json_text(&message), "Docs", "{message}");
    assert_eq!(message[click], open_url(version), "{message}");
    assert_eq!(message[hover]["action"], json!("show_text"), "{message}");
    assert_eq!(tooltip_text(&message[hover]), "Open the docs", "{message}");
    assert!(message.get(stale_click).is_none(), "{message}");
    assert!(message.get(stale_hover).is_none(), "{message}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(plugin_message_events_follow_the_client_version;
    p47 = 47, p764 = 764, p765 = 765, p769 = 769, p774 = 774);

async fn switch_denial_reaches_the_player_as_sent(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("a")
                .backend(backend_a.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("b")
                .backend(backend_b.addr())
                .network("main"),
        )
        .plugin(deny_server("b", "B is full"))
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend_a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    player.switch_server(ServerId::new("b")).await.unwrap();

    let raw = session.expect_system_message(T).await.unwrap();
    let message = component_json(&raw, version).unwrap();
    assert_eq!(json_text(&message), "B is full", "{message}");
    assert_eq!(message["color"], json!("red"), "{message}");
    assert_eq!(player.current_server(), Some(ServerId::new("a")));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, switch_denial_reaches_the_player_as_sent);

fn rich_motd() -> Value {
    json!({
        "text": "Welcome",
        "color": "gold",
        "clickEvent": { "action": "open_url", "value": DOCS_URL },
        "hoverEvent": { "action": "show_text", "contents": "Read me" },
    })
}

fn backend_status(version: ProtocolVersion) -> Value {
    json!({
        "version": { "name": "Harness", "protocol": version.0 },
        "players": { "max": 20, "online": 3 },
        "description": rich_motd(),
        "enforcesSecureChat": true,
    })
}

async fn edited_rich_motd_keeps_its_events(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .status(backend_status(version))
        .spawn()
        .await
        .unwrap();
    let motd = ScriptedPlugin::new("motd").on::<ProxyPingEvent>(EventPriority::NORMAL, |event| {
        let description = event.response.description.clone();
        event.response.description =
            description.append(Component::text(" and more").hover(HoverEvent::show_text("Added")));
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(motd)
        .start()
        .await
        .unwrap();

    let status = proxy.client(version).status().await.unwrap();
    let description = &status.json["description"];

    let (click, hover) = event_keys(version);
    assert_eq!(json_text(description), "Welcome and more", "{description}");
    assert_eq!(description["color"], json!("gold"), "{description}");
    assert_eq!(description[click], open_url(version), "{description}");
    assert_eq!(description[hover]["action"], json!("show_text"));
    assert_eq!(
        tooltip_text(&description[hover]),
        "Read me",
        "{description}"
    );
    let added = &description["extra"][0];
    assert_eq!(tooltip_text(&added[hover]), "Added", "{description}");
    assert_eq!(status.json["enforcesSecureChat"], json!(true));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, edited_rich_motd_keeps_its_events);

async fn untouched_rich_motd_is_relayed_verbatim(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .status(backend_status(version))
        .spawn()
        .await
        .unwrap();
    let counts =
        ScriptedPlugin::new("counts").on::<ProxyPingEvent>(EventPriority::NORMAL, |event| {
            event.response.max_players = 99;
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(counts)
        .start()
        .await
        .unwrap();

    let status = proxy.client(version).status().await.unwrap();

    assert_eq!(status.json["description"], rich_motd());
    assert_eq!(status.json["players"]["max"], json!(99));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, untouched_rich_motd_is_relayed_verbatim);
