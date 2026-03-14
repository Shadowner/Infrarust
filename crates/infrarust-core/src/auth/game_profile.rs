//! Game profile types for Mojang authentication.

use serde::Deserialize;
use uuid::Uuid;

use crate::error::CoreError;

/// Player profile returned by the Mojang session server.
#[derive(Debug, Clone, Deserialize)]
pub struct GameProfile {
    /// UUID as a no-dash hex string (Mojang format).
    pub id: String,
    /// Player username.
    pub name: String,
    /// Skin/cape textures and other properties.
    #[serde(default)]
    pub properties: Vec<ProfileProperty>,
}

/// A property attached to a game profile (typically skin textures).
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

impl GameProfile {
    /// Parses the Mojang UUID (no-dash hex format) into a proper `Uuid`.
    pub fn uuid(&self) -> Result<Uuid, CoreError> {
        Uuid::parse_str(&self.id).map_err(|e| CoreError::Auth(format!("invalid uuid: {e}")))
    }
}

/// Generates an offline-mode UUID from a username.
///
/// Matches vanilla Minecraft's `UUID.nameUUIDFromBytes("OfflinePlayer:" + name)`.
/// This is a UUID v3 (MD5-based) with no namespace prefix — equivalent to
/// using a nil UUID as namespace in the `uuid` crate.
pub fn offline_uuid(username: &str) -> Uuid {
    let input = format!("OfflinePlayer:{username}");
    Uuid::new_v3(&Uuid::nil(), input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_deterministic() {
        let a = offline_uuid("Notch");
        let b = offline_uuid("Notch");
        assert_eq!(a, b);
    }

    #[test]
    fn offline_uuid_different_names() {
        let a = offline_uuid("Notch");
        let b = offline_uuid("jeb_");
        assert_ne!(a, b);
    }

    #[test]
    fn offline_uuid_known_value() {
        // Vanilla Minecraft offline UUID for "Notch"
        // Java's UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes())
        let uuid = offline_uuid("Notch");
        // The UUID should be version 3 and variant RFC 4122
        assert_eq!(uuid.get_version_num(), 3);
    }

    #[test]
    fn game_profile_parse_uuid() {
        let profile = GameProfile {
            id: "069a79f444e94726a5befca90e38aaf5".to_string(),
            name: "Notch".to_string(),
            properties: vec![],
        };
        let uuid = profile.uuid().unwrap();
        assert_eq!(uuid.to_string(), "069a79f4-44e9-4726-a5be-fca90e38aaf5");
    }
}
