#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Tests for core ↔ API ping response conversion.

use infrarust_api::events::proxy::PingResponse;
use infrarust_api::types::{ClickEvent, Component, HoverEvent, NamedColor, ProtocolVersion};

use infrarust_core::event_bus::conversion::{apply_api_to_core, core_to_api_ping_response};
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

    assert_eq!(api.description.as_text(), Some("A Minecraft Server"));
    assert_eq!(api.description.style.color, Some(NamedColor::Gold.into()));
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
    assert_eq!(api.description.as_text(), Some(""));
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

    apply_api_to_core(&mut core, &api, ProtocolVersion::MINECRAFT_1_20_2);

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
    assert_eq!(api.description.as_text(), Some("Plain text MOTD"));
    assert!(api.description.style.color.is_none());
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
    assert_eq!(api.description.as_text(), Some("Hello"));
    assert_eq!(api.description.style.color, Some(NamedColor::Green.into()));
    assert_eq!(api.description.style.bold, Some(true));
    assert_eq!(api.description.children.len(), 1);
    assert_eq!(api.description.children[0].as_text(), Some(" World"));
    assert_eq!(
        api.description.children[0].style.color,
        Some(NamedColor::White.into())
    );
}

#[test]
fn test_description_round_trips_through_the_core_response() {
    let original = Component::text("Hello")
        .color("gold")
        .bold()
        .click(ClickEvent::OpenUrl("https://example.com".into()))
        .hover(HoverEvent::show_text(Component::text("tip").italic()))
        .append(Component::text(" World").color("white"));
    let api = PingResponse::new(
        original.clone(),
        1,
        0,
        ProtocolVersion::new(769),
        "1.21.4".to_string(),
        None,
    );

    for client in [
        ProtocolVersion::MINECRAFT_1_8,
        ProtocolVersion::MINECRAFT_1_21_4,
        ProtocolVersion::MINECRAFT_1_21_11,
    ] {
        let mut core = make_core_response();
        apply_api_to_core(&mut core, &api, client);
        let back = core_to_api_ping_response(&core);
        assert_eq!(back.description, original, "{client}");
    }
}
