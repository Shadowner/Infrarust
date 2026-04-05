//! Conversion between core `ServerPingResponse` (`serde_json–based`) and
//! API `PingResponse` (typed fields) for the `ProxyPingEvent`.

use infrarust_api::events::proxy::PingResponse;
use infrarust_api::types::{Component, ProtocolVersion};

use crate::status::response::ServerPingResponse;

/// Converts the core status response into the typed API representation.
///
/// The core type stores the MOTD as a `serde_json::Value` (string, object,
/// or array) while the API type uses a structured [`Component`].
pub fn core_to_api_ping_response(core: &ServerPingResponse) -> PingResponse {
    PingResponse::new(
        json_value_to_component(&core.description),
        core.players.max,
        core.players.online,
        ProtocolVersion::new(core.version.protocol),
        core.version.name.clone(),
        core.favicon.clone(),
    )
}

/// Merges modifications from the API `PingResponse` back into the core
/// response, preserving `extra` fields (Forge/Fabric metadata).
pub fn apply_api_to_core(core: &mut ServerPingResponse, api: &PingResponse) {
    core.description = component_to_json_value(&api.description);
    core.players.max = api.max_players;
    core.players.online = api.online_players;
    core.version.name.clone_from(&api.version_name);
    core.version.protocol = api.protocol_version.raw();
    core.favicon.clone_from(&api.favicon);
}

/// Converts a `serde_json::Value` (Minecraft chat JSON) into a [`Component`].
///
/// Handles the three common MOTD formats:
/// - Plain string: `"Hello"` → `Component::text("Hello")`
/// - Chat object: `{"text":"Hello","color":"green"}` → structured Component
/// - Array: `[{"text":"a"},{"text":"b"}]` → Component with extras
/// - Anything else: serialized as text fallback
pub fn json_value_to_component(value: &serde_json::Value) -> Component {
    match value {
        serde_json::Value::String(s) => Component::text(s.as_str()),
        serde_json::Value::Object(map) => {
            let text = map.get("text").and_then(|v| v.as_str()).unwrap_or_default();
            let mut component = Component::text(text);

            if let Some(color) = map.get("color").and_then(|v| v.as_str()) {
                component = component.color(color);
            }
            if map.get("bold").and_then(serde_json::Value::as_bool) == Some(true) {
                component = component.bold();
            }
            if map.get("italic").and_then(serde_json::Value::as_bool) == Some(true) {
                component = component.italic();
            }
            if map.get("underlined").and_then(serde_json::Value::as_bool) == Some(true) {
                component = component.underlined();
            }
            if map
                .get("strikethrough")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                component = component.strikethrough();
            }
            if map.get("obfuscated").and_then(serde_json::Value::as_bool) == Some(true) {
                component = component.obfuscated();
            }

            // Recurse into "extra" array
            if let Some(serde_json::Value::Array(extras)) = map.get("extra") {
                for extra in extras {
                    component = component.append(json_value_to_component(extra));
                }
            }

            component
        }
        serde_json::Value::Array(arr) => {
            // Array of components: first is root, rest are extras
            let mut iter = arr.iter();
            let mut root = iter.next().map(json_value_to_component).unwrap_or_default();
            for extra in iter {
                root = root.append(json_value_to_component(extra));
            }
            root
        }
        // Fallback: serialize to string
        other => Component::text(other.to_string()),
    }
}

/// Converts a [`Component`] into Minecraft chat JSON (`serde_json::Value`).
pub fn component_to_json_value(component: &Component) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    map.insert(
        "text".to_string(),
        serde_json::Value::String(component.text.clone()),
    );

    if let Some(ref color) = component.color {
        map.insert(
            "color".to_string(),
            serde_json::Value::String(color.clone()),
        );
    }
    if component.bold == Some(true) {
        map.insert("bold".to_string(), serde_json::Value::Bool(true));
    }
    if component.italic == Some(true) {
        map.insert("italic".to_string(), serde_json::Value::Bool(true));
    }
    if component.underlined == Some(true) {
        map.insert("underlined".to_string(), serde_json::Value::Bool(true));
    }
    if component.strikethrough == Some(true) {
        map.insert("strikethrough".to_string(), serde_json::Value::Bool(true));
    }
    if component.obfuscated == Some(true) {
        map.insert("obfuscated".to_string(), serde_json::Value::Bool(true));
    }

    if !component.extra.is_empty() {
        let extras: Vec<serde_json::Value> = component
            .extra
            .iter()
            .map(component_to_json_value)
            .collect();
        map.insert("extra".to_string(), serde_json::Value::Array(extras));
    }

    serde_json::Value::Object(map)
}

/// Converts `infrarust_server_manager::ServerState` to the API's
/// `infrarust_api::services::server_manager::ServerState`.
pub const fn convert_server_state(
    sm: infrarust_server_manager::ServerState,
) -> infrarust_api::services::server_manager::ServerState {
    use infrarust_api::services::server_manager::ServerState as ApiState;
    use infrarust_server_manager::ServerState as SmState;

    match sm {
        SmState::Online => ApiState::Online,
        SmState::Sleeping => ApiState::Sleeping,
        SmState::Starting => ApiState::Starting,
        SmState::Stopping => ApiState::Stopping,
        SmState::Crashed => ApiState::Crashed,
        _ => ApiState::Offline,
    }
}

/// Converts protocol `ConnectionState` to API `ConnectionState`.
pub fn protocol_state_to_api(
    state: infrarust_protocol::version::ConnectionState,
) -> infrarust_api::event::ConnectionState {
    match state {
        infrarust_protocol::version::ConnectionState::Handshake => {
            infrarust_api::event::ConnectionState::Handshake
        }
        infrarust_protocol::version::ConnectionState::Status => {
            infrarust_api::event::ConnectionState::Status
        }
        infrarust_protocol::version::ConnectionState::Login => {
            infrarust_api::event::ConnectionState::Login
        }
        infrarust_protocol::version::ConnectionState::Config => {
            infrarust_api::event::ConnectionState::Configuration
        }
        infrarust_protocol::version::ConnectionState::Play => {
            infrarust_api::event::ConnectionState::Play
        }
    }
}

/// Converts protocol `Direction` to API `PacketDirection`.
pub fn protocol_direction_to_api(
    direction: infrarust_protocol::version::Direction,
) -> infrarust_api::events::packet::PacketDirection {
    match direction {
        infrarust_protocol::version::Direction::Serverbound => {
            infrarust_api::events::packet::PacketDirection::Serverbound
        }
        infrarust_protocol::version::Direction::Clientbound => {
            infrarust_api::events::packet::PacketDirection::Clientbound
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_json_string_to_component() {
        let value = serde_json::json!("Hello World");
        let component = json_value_to_component(&value);
        assert_eq!(component.text, "Hello World");
        assert!(component.color.is_none());
        assert!(component.extra.is_empty());
    }

    #[test]
    fn test_json_object_to_component() {
        let value = serde_json::json!({"text": "Hello", "color": "gold", "bold": true});
        let component = json_value_to_component(&value);
        assert_eq!(component.text, "Hello");
        assert_eq!(component.color.as_deref(), Some("gold"));
        assert_eq!(component.bold, Some(true));
    }

    #[test]
    fn test_json_object_with_extra() {
        let value = serde_json::json!({"text": "A", "extra": [{"text": "B"}, {"text": "C", "color": "red"}]});
        let component = json_value_to_component(&value);
        assert_eq!(component.text, "A");
        assert_eq!(component.extra.len(), 2);
        assert_eq!(component.extra[0].text, "B");
        assert_eq!(component.extra[1].text, "C");
        assert_eq!(component.extra[1].color.as_deref(), Some("red"));
    }

    #[test]
    fn test_json_array_to_component() {
        let value = serde_json::json!([{"text": "X"}, {"text": "Y"}]);
        let component = json_value_to_component(&value);
        assert_eq!(component.text, "X");
        assert_eq!(component.extra.len(), 1);
        assert_eq!(component.extra[0].text, "Y");
    }

    #[test]
    fn test_component_to_json_roundtrip() {
        let original = Component::text("Hello").color("green").bold();
        let json = component_to_json_value(&original);
        let back = json_value_to_component(&json);
        assert_eq!(back.text, "Hello");
        assert_eq!(back.color.as_deref(), Some("green"));
        assert_eq!(back.bold, Some(true));
    }

    #[test]
    fn test_core_to_api_full_response() {
        use crate::status::response::{PingPlayerSample, PingPlayers, PingVersion};

        let core = ServerPingResponse {
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
            description: serde_json::json!({"text": "A Minecraft Server"}),
            favicon: Some("data:image/png;base64,abc".to_string()),
            extra: serde_json::Map::new(),
        };

        let api = core_to_api_ping_response(&core);
        assert_eq!(api.description.text, "A Minecraft Server");
        assert_eq!(api.max_players, 100);
        assert_eq!(api.online_players, 42);
        assert_eq!(api.protocol_version.raw(), 769);
        assert_eq!(api.version_name, "1.21.4");
        assert_eq!(api.favicon.as_deref(), Some("data:image/png;base64,abc"));
    }

    #[test]
    fn test_apply_api_preserves_extra() {
        use crate::status::response::{PingPlayers, PingVersion};

        let mut core = ServerPingResponse {
            version: PingVersion {
                name: "1.21.4".to_string(),
                protocol: 769,
            },
            players: PingPlayers {
                max: 100,
                online: 42,
                sample: vec![],
            },
            description: serde_json::json!({"text": "Original"}),
            favicon: None,
            extra: {
                let mut m = serde_json::Map::new();
                m.insert(
                    "forgeData".to_string(),
                    serde_json::json!({"mods": ["forge"]}),
                );
                m
            },
        };

        let api = PingResponse::new(
            Component::text("Modified"),
            200,
            99,
            ProtocolVersion::new(769),
            "1.21.4".to_string(),
            Some("data:image/png;base64,new".to_string()),
        );

        apply_api_to_core(&mut core, &api);

        // Modified fields
        assert_eq!(core.description["text"].as_str().unwrap(), "Modified");
        assert_eq!(core.players.max, 200);
        assert_eq!(core.players.online, 99);
        assert_eq!(core.favicon.as_deref(), Some("data:image/png;base64,new"));

        // Extra fields preserved
        assert!(core.extra.contains_key("forgeData"));
    }
}
