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
        Component::from_json_value(&core.description),
        core.players.max,
        core.players.online,
        ProtocolVersion::new(core.version.protocol),
        core.version.name.clone(),
        core.favicon.clone(),
    )
}

pub fn apply_api_to_core(
    core: &mut ServerPingResponse,
    api: &PingResponse,
    client: ProtocolVersion,
) {
    core.description = api.description.to_json_value_for(client);
    apply_api_scalars_to_core(core, api);
}

pub fn apply_api_scalars_to_core(core: &mut ServerPingResponse, api: &PingResponse) {
    core.players.max = api.max_players;
    core.players.online = api.online_players;
    core.version.name.clone_from(&api.version_name);
    core.version.protocol = api.protocol_version.raw();
    core.favicon.clone_from(&api.favicon);
}

/// Folds a (possibly plugin-modified) `ProxyPingEvent` result back into the core
/// response.
pub fn merge_ping_event(
    core: &mut ServerPingResponse,
    sent_description: &Component,
    api: &PingResponse,
    client: ProtocolVersion,
) {
    if api.description == *sent_description {
        apply_api_scalars_to_core(core, api);
    } else {
        apply_api_to_core(core, api, client);
    }
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
        assert_eq!(api.description.as_text(), Some("A Minecraft Server"));
        assert_eq!(api.max_players, 100);
        assert_eq!(api.online_players, 42);
        assert_eq!(api.protocol_version.raw(), 769);
        assert_eq!(api.version_name, "1.21.4");
        assert_eq!(api.favicon.as_deref(), Some("data:image/png;base64,abc"));
    }

    fn rich_response() -> ServerPingResponse {
        use crate::status::response::{PingPlayers, PingVersion};
        ServerPingResponse {
            version: PingVersion {
                name: "1.21.4".to_string(),
                protocol: 769,
            },
            players: PingPlayers {
                max: 100,
                online: 42,
                sample: vec![],
            },
            description: serde_json::json!({
                "text": "Welcome",
                "color": "gold",
                "hoverEvent": {"action": "show_text", "value": "tooltip"},
                "clickEvent": {"action": "open_url", "value": "https://example.com"},
                "extra": [{"text": "!", "font": "minecraft:uniform"}]
            }),
            favicon: None,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn merge_ping_event_preserves_rich_motd_when_unmodified() {
        // Regression: with no plugin modification, a rich relayed MOTD must NOT
        // be degraded through the lossy Component round-trip.
        let mut core = rich_response();
        let original = core.description.clone();

        let api = core_to_api_ping_response(&core);
        let sent = api.description.clone();
        // Simulate the event returning the response unmodified.
        merge_ping_event(&mut core, &sent, &api, ProtocolVersion::MINECRAFT_1_21_11);

        assert_eq!(
            core.description, original,
            "rich MOTD must survive untouched"
        );
        assert!(core.description.get("hoverEvent").is_some());
        assert!(core.description.get("clickEvent").is_some());
    }

    #[test]
    fn merge_ping_event_applies_modified_description() {
        let mut core = rich_response();

        let mut api = core_to_api_ping_response(&core);
        let sent = api.description.clone();
        api.description = Component::text("Plugin MOTD");

        merge_ping_event(&mut core, &sent, &api, ProtocolVersion::MINECRAFT_1_20_2);
        assert_eq!(core.description["text"].as_str().unwrap(), "Plugin MOTD");
    }

    #[test]
    fn edited_rich_motd_keeps_events_in_the_client_shape() {
        let mut api = core_to_api_ping_response(&rich_response());
        let sent = api.description.clone();
        api.description = sent.clone().append(Component::text("?"));

        let mut old = rich_response();
        merge_ping_event(&mut old, &sent, &api, ProtocolVersion::MINECRAFT_1_21_4);
        assert_eq!(
            old.description,
            serde_json::json!({
                "text": "Welcome",
                "color": "gold",
                "hoverEvent": {"action": "show_text", "contents": "tooltip"},
                "clickEvent": {"action": "open_url", "value": "https://example.com"},
                "extra": [{"text": "!", "font": "minecraft:uniform"}, "?"]
            })
        );

        let mut new = rich_response();
        merge_ping_event(&mut new, &sent, &api, ProtocolVersion::MINECRAFT_1_21_11);
        assert_eq!(
            new.description,
            serde_json::json!({
                "text": "Welcome",
                "color": "gold",
                "hover_event": {"action": "show_text", "value": "tooltip"},
                "click_event": {"action": "open_url", "url": "https://example.com"},
                "extra": [{"text": "!", "font": "minecraft:uniform"}, "?"]
            })
        );
    }

    #[test]
    fn merge_ping_event_applies_scalar_changes_without_touching_description() {
        // A plugin changing only player counts must not degrade the description.
        let mut core = rich_response();
        let original = core.description.clone();

        let mut api = core_to_api_ping_response(&core);
        let sent = api.description.clone();
        api.max_players = 5;
        api.online_players = 1;

        merge_ping_event(&mut core, &sent, &api, ProtocolVersion::MINECRAFT_1_21_11);
        assert_eq!(core.players.max, 5);
        assert_eq!(core.players.online, 1);
        assert_eq!(core.description, original, "description must stay opaque");
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

        apply_api_to_core(&mut core, &api, ProtocolVersion::MINECRAFT_1_20_2);

        // Modified fields
        assert_eq!(core.description["text"].as_str().unwrap(), "Modified");
        assert_eq!(core.players.max, 200);
        assert_eq!(core.players.online, 99);
        assert_eq!(core.favicon.as_deref(), Some("data:image/png;base64,new"));

        // Extra fields preserved
        assert!(core.extra.contains_key("forgeData"));
    }
}
