#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Tests for core ↔ API ping response conversion.

use infrarust_api::events::proxy::PingResponse;
use infrarust_api::types::{Component, ProtocolVersion};

use infrarust_core::event_bus::conversion::{
    apply_api_to_core, component_to_json_value, core_to_api_ping_response, json_value_to_component,
};
use infrarust_core::status::response::{
    PingPlayerSample, PingPlayers, PingVersion, ServerPingResponse,
};

fn make_core_response() -> ServerPingResponse {
    ServerPingResponse {
        version: PingVersion {
            name: "1.21.4".to_string(),
            protocol: 769,
        },
        players: PingPlayers {
            max: 100,
            online: 42,
            sample: vec![PingPlayerSample {
                name: "Notch".to_string(),
                id: "069a79f4-44e9-4726-a5be-fca90e38aaf5".to_string(),
            }],
        },
        description: serde_json::json!({"text": "A Minecraft Server", "color": "gold"}),
        favicon: Some("data:image/png;base64,abc".to_string()),
        extra: serde_json::Map::new(),
    }
}

#[test]
fn test_core_to_api_full_response() {
    let core = make_core_response();
    let api = core_to_api_ping_response(&core);

    assert_eq!(api.description.text, "A Minecraft Server");
    assert_eq!(api.description.color.as_deref(), Some("gold"));
    assert_eq!(api.max_players, 100);
    assert_eq!(api.online_players, 42);
    assert_eq!(api.protocol_version.raw(), 769);
    assert_eq!(api.version_name, "1.21.4");
    assert_eq!(api.favicon.as_deref(), Some("data:image/png;base64,abc"));
}

#[test]
fn test_core_to_api_minimal_response() {
    let core = ServerPingResponse {
        version: PingVersion {
            name: "Infrarust".to_string(),
            protocol: 769,
        },
        players: PingPlayers {
            max: 0,
            online: 0,
            sample: vec![],
        },
        description: serde_json::json!({"text": ""}),
        favicon: None,
        extra: serde_json::Map::new(),
    };

    let api = core_to_api_ping_response(&core);
    assert_eq!(api.description.text, "");
    assert_eq!(api.max_players, 0);
    assert!(api.favicon.is_none());
}

#[test]
fn test_apply_api_preserves_extra() {
    let mut core = make_core_response();
    core.extra.insert(
        "forgeData".to_string(),
        serde_json::json!({"mods": [{"modId": "forge"}]}),
    );
    core.extra.insert(
        "enforcesSecureChat".to_string(),
        serde_json::Value::Bool(true),
    );

    let api = PingResponse::new(
        Component::text("Modified MOTD"),
        200,
        99,
        ProtocolVersion::new(769),
        "Custom 1.21".to_string(),
        Some("data:image/png;base64,new".to_string()),
    );

    apply_api_to_core(&mut core, &api);

    // Modified fields
    assert_eq!(core.description["text"].as_str().unwrap(), "Modified MOTD");
    assert_eq!(core.players.max, 200);
    assert_eq!(core.players.online, 99);
    assert_eq!(core.version.name, "Custom 1.21");
    assert_eq!(core.favicon.as_deref(), Some("data:image/png;base64,new"));

    // Extra fields preserved!
    assert!(core.extra.contains_key("forgeData"));
    assert!(core.extra.contains_key("enforcesSecureChat"));
}

#[test]
fn test_core_to_api_string_description() {
    let core = ServerPingResponse {
        version: PingVersion {
            name: "1.21".to_string(),
            protocol: 767,
        },
        players: PingPlayers {
            max: 0,
            online: 0,
            sample: vec![],
        },
        description: serde_json::Value::String("Plain text MOTD".to_string()),
        favicon: None,
        extra: serde_json::Map::new(),
    };

    let api = core_to_api_ping_response(&core);
    assert_eq!(api.description.text, "Plain text MOTD");
    assert!(api.description.color.is_none());
}

#[test]
fn test_core_to_api_object_description() {
    let core = ServerPingResponse {
        version: PingVersion {
            name: "1.21".to_string(),
            protocol: 767,
        },
        players: PingPlayers {
            max: 0,
            online: 0,
            sample: vec![],
        },
        description: serde_json::json!({
            "text": "Hello",
            "color": "green",
            "bold": true,
            "extra": [{"text": " World", "color": "white"}]
        }),
        favicon: None,
        extra: serde_json::Map::new(),
    };

    let api = core_to_api_ping_response(&core);
    assert_eq!(api.description.text, "Hello");
    assert_eq!(api.description.color.as_deref(), Some("green"));
    assert_eq!(api.description.bold, Some(true));
    assert_eq!(api.description.extra.len(), 1);
    assert_eq!(api.description.extra[0].text, " World");
    assert_eq!(api.description.extra[0].color.as_deref(), Some("white"));
}

#[test]
fn test_component_to_json_roundtrip() {
    let original = Component::text("Hello")
        .color("gold")
        .bold()
        .append(Component::text(" World").color("white"));

    let json = component_to_json_value(&original);
    let back = json_value_to_component(&json);

    assert_eq!(back.text, "Hello");
    assert_eq!(back.color.as_deref(), Some("gold"));
    assert_eq!(back.bold, Some(true));
    assert_eq!(back.extra.len(), 1);
    assert_eq!(back.extra[0].text, " World");
    assert_eq!(back.extra[0].color.as_deref(), Some("white"));
}
