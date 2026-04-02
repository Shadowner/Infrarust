//! Status ping response types.
//!
//! `ServerPingResponse` is the structured representation of the JSON returned
//! by a Minecraft server's status endpoint. It can be constructed from a real
//! backend relay or synthesized for contextual MOTDs (sleeping, starting, etc.).

use infrarust_config::MotdEntry;
use infrarust_protocol::CURRENT_MC_PROTOCOL;
use serde::{Deserialize, Serialize};

/// Parsed Minecraft status response.
///
/// Corresponds to the JSON returned by a modern (1.7+) Minecraft server.
/// The `extra` field captures unknown keys (Forge's `forgeData`, Fabric's
/// `modinfo`, etc.) so they are preserved during relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPingResponse {
    pub version: PingVersion,
    pub players: PingPlayers,
    /// The MOTD. May be a simple string (`"Hello"`), a chat component object
    /// (`{"text":"Hello","color":"green"}`), or an array of components.
    /// Stored as opaque `serde_json::Value` to handle all variants.
    pub description: serde_json::Value,
    /// Favicon as a data URI: `"data:image/png;base64,..."`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    /// Extra fields not part of the vanilla protocol (Forge, Fabric, etc.).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingVersion {
    pub name: String,
    pub protocol: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingPlayers {
    pub max: i32,
    pub online: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample: Vec<PingPlayerSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingPlayerSample {
    pub name: String,
    pub id: String,
}

impl ServerPingResponse {
    /// Builds a synthetic response for a given state (sleeping, starting, etc.).
    pub fn synthetic(
        description: &str,
        favicon: Option<&str>,
        version_name: Option<&str>,
        max_players: Option<i32>,
    ) -> Self {
        Self {
            version: PingVersion {
                name: version_name.unwrap_or("Infrarust").to_string(),
                protocol: CURRENT_MC_PROTOCOL,
            },
            players: PingPlayers {
                max: max_players.unwrap_or(0),
                online: 0,
                sample: vec![],
            },
            description: serde_json::json!({"text": description}),
            favicon: favicon.map(String::from),
            extra: serde_json::Map::new(),
        }
    }

    /// Applies config overrides from a `MotdEntry` onto this response.
    ///
    /// `text` always overrides the description. Other fields override only
    /// when present (`Some`).
    pub fn apply_overrides(&mut self, motd: &MotdEntry) {
        self.description = serde_json::json!({"text": &motd.text});
        if let Some(ref fav) = motd.favicon {
            self.favicon = Some(fav.clone());
        }
        if let Some(ref name) = motd.version_name {
            self.version.name.clone_from(name);
        }
        if let Some(protocol) = motd.version_protocol {
            self.version.protocol = protocol;
        }
        if let Some(max) = motd.max_players {
            self.players.max = max.cast_signed();
        }
    }

    /// Serializes to a JSON string for the `CStatusResponse` packet.
    ///
    /// # Errors
    /// Returns `serde_json::Error` if serialization fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const VANILLA_JSON: &str = r#"{
        "version": {"name": "1.21.4", "protocol": 769},
        "players": {"max": 100, "online": 42, "sample": [
            {"name": "Notch", "id": "069a79f4-44e9-4726-a5be-fca90e38aaf5"}
        ]},
        "description": {"text": "A Minecraft Server"}
    }"#;

    const PAPER_JSON: &str = r#"{
        "version": {"name": "1.21.4", "protocol": 769},
        "players": {"max": 100, "online": 5},
        "description": {"text": "Paper Server"},
        "enforcesSecureChat": true,
        "previewsChat": false
    }"#;

    const FORGE_JSON: &str = r#"{
        "version": {"name": "1.21.4", "protocol": 769},
        "players": {"max": 20, "online": 3},
        "description": "Forge Server",
        "forgeData": {
            "channels": [{"res": "fml:handshake", "version": "1.0"}],
            "mods": [{"modId": "forge", "modmarker": "47.0.0"}]
        }
    }"#;

    #[test]
    fn test_parse_vanilla_response() {
        let resp: ServerPingResponse = serde_json::from_str(VANILLA_JSON).unwrap();
        assert_eq!(resp.version.name, "1.21.4");
        assert_eq!(resp.version.protocol, 769);
        assert_eq!(resp.players.max, 100);
        assert_eq!(resp.players.online, 42);
        assert_eq!(resp.players.sample.len(), 1);
        assert_eq!(resp.players.sample[0].name, "Notch");
        assert!(resp.extra.is_empty());
    }

    #[test]
    fn test_parse_paper_response() {
        let resp: ServerPingResponse = serde_json::from_str(PAPER_JSON).unwrap();
        assert_eq!(resp.players.online, 5);
        assert!(resp.extra.contains_key("enforcesSecureChat"));
        assert!(resp.extra.contains_key("previewsChat"));
    }

    #[test]
    fn test_parse_forge_response() {
        let resp: ServerPingResponse = serde_json::from_str(FORGE_JSON).unwrap();
        assert_eq!(resp.players.max, 20);
        assert!(resp.extra.contains_key("forgeData"));
        // Description is a plain string, not an object
        assert_eq!(resp.description.as_str().unwrap(), "Forge Server");
    }

    #[test]
    fn test_parse_minimal_response() {
        let json = r#"{"version":{"name":"1.21","protocol":767},"players":{"max":0,"online":0},"description":""}"#;
        let resp: ServerPingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.version.protocol, 767);
        assert!(resp.players.sample.is_empty());
    }

    #[test]
    fn test_parse_string_description() {
        let json = r#"{"version":{"name":"1.21","protocol":767},"players":{"max":0,"online":0},"description":"Hello"}"#;
        let resp: ServerPingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.description.as_str().unwrap(), "Hello");
    }

    #[test]
    fn test_parse_object_description() {
        let json = r#"{"version":{"name":"1.21","protocol":767},"players":{"max":0,"online":0},"description":{"text":"Hello","color":"green"}}"#;
        let resp: ServerPingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.description["text"].as_str().unwrap(), "Hello");
        assert_eq!(resp.description["color"].as_str().unwrap(), "green");
    }

    #[test]
    fn test_parse_array_description() {
        let json = r#"{"version":{"name":"1.21","protocol":767},"players":{"max":0,"online":0},"description":[{"text":"a"},{"text":"b"}]}"#;
        let resp: ServerPingResponse = serde_json::from_str(json).unwrap();
        assert!(resp.description.is_array());
        assert_eq!(resp.description[0]["text"].as_str().unwrap(), "a");
    }

    #[test]
    fn test_serialize_roundtrip() {
        let resp: ServerPingResponse = serde_json::from_str(VANILLA_JSON).unwrap();
        let json = resp.to_json().unwrap();
        let resp2: ServerPingResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.version.protocol, resp2.version.protocol);
        assert_eq!(resp.players.online, resp2.players.online);
        assert_eq!(resp.players.sample.len(), resp2.players.sample.len());
    }

    #[test]
    fn test_forge_extra_roundtrip() {
        let resp: ServerPingResponse = serde_json::from_str(FORGE_JSON).unwrap();
        let json = resp.to_json().unwrap();
        let resp2: ServerPingResponse = serde_json::from_str(&json).unwrap();
        assert!(resp2.extra.contains_key("forgeData"));
    }

    #[test]
    fn test_synthetic_sleeping() {
        let resp = ServerPingResponse::synthetic("\u{00a7}7Server sleeping", None, None, None);
        assert_eq!(resp.version.name, "Infrarust");
        assert_eq!(resp.players.max, 0);
        assert_eq!(resp.players.online, 0);
        assert!(resp.favicon.is_none());
        assert!(resp.extra.is_empty());
        let json = resp.to_json().unwrap();
        assert!(json.contains("Server sleeping"));
    }

    #[test]
    fn test_apply_overrides_text() {
        let mut resp: ServerPingResponse = serde_json::from_str(VANILLA_JSON).unwrap();
        let entry = MotdEntry {
            text: "Custom MOTD".to_string(),
            favicon: None,
            version_name: None,
            version_protocol: None,
            max_players: None,
        };
        resp.apply_overrides(&entry);
        assert_eq!(resp.description["text"].as_str().unwrap(), "Custom MOTD");
        // Other fields unchanged
        assert_eq!(resp.players.online, 42);
        assert_eq!(resp.version.name, "1.21.4");
    }

    #[test]
    fn test_apply_overrides_partial() {
        let mut resp: ServerPingResponse = serde_json::from_str(VANILLA_JSON).unwrap();
        let entry = MotdEntry {
            text: "New MOTD".to_string(),
            favicon: None,
            version_name: None,
            version_protocol: None,
            max_players: Some(200),
        };
        resp.apply_overrides(&entry);
        assert_eq!(resp.players.max, 200);
        assert_eq!(resp.version.name, "1.21.4"); // unchanged
    }

    #[test]
    fn test_apply_overrides_favicon() {
        let mut resp: ServerPingResponse = serde_json::from_str(VANILLA_JSON).unwrap();
        assert!(resp.favicon.is_none());
        let entry = MotdEntry {
            text: "X".to_string(),
            favicon: Some("data:image/png;base64,abc".to_string()),
            version_name: None,
            version_protocol: None,
            max_players: None,
        };
        resp.apply_overrides(&entry);
        assert_eq!(resp.favicon.as_deref(), Some("data:image/png;base64,abc"));
    }
}
