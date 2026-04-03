//! Domain types for the Infrarust plugin API.
//!
//! This module contains the core data types used throughout the API:
//! player identifiers, server identifiers, rich text components,
//! protocol versions, raw packets, and permissions.

mod component;
mod extensions;
mod permission;
mod player_id;
mod protocol_version;
mod raw_packet;
mod server_id;

pub use component::{ClickEvent, Component, HoverEvent, TitleData, format_placeholders};
pub use extensions::Extensions;
pub use permission::Permission;
pub use player_id::PlayerId;
pub use protocol_version::ProtocolVersion;
pub use raw_packet::RawPacket;
pub use server_id::{ServerAddress, ServerId, ServerInfo};

/// A player's Mojang game profile.
///
/// Contains the UUID, username, and profile properties (typically skin data)
/// as received during the login handshake.
#[derive(Debug, Clone)]
pub struct GameProfile {
    /// The player's Mojang UUID.
    pub uuid: uuid::Uuid,
    /// The player's username.
    pub username: String,
    /// Profile properties (e.g. skin textures).
    pub properties: Vec<ProfileProperty>,
}

impl GameProfile {
    /// Returns `true` if this profile has a signed `textures` property,
    /// indicating the player was authenticated by Mojang (online mode).
    pub fn is_mojang_authenticated(&self) -> bool {
        self.properties
            .iter()
            .any(|p| p.name == "textures" && p.signature.is_some())
    }
}

/// A single property on a [`GameProfile`].
///
/// Typically contains skin texture data signed by Mojang.
#[derive(Debug, Clone)]
pub struct ProfileProperty {
    /// Property name (e.g. `"textures"`).
    pub name: String,
    /// Base64-encoded property value.
    pub value: String,
    /// Optional Mojang signature for this property.
    pub signature: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn game_profile_construction() {
        let profile = GameProfile {
            uuid: uuid::Uuid::nil(),
            username: "Steve".into(),
            properties: vec![ProfileProperty {
                name: "textures".into(),
                value: "base64data".into(),
                signature: Some("sig".into()),
            }],
        };
        assert_eq!(profile.username, "Steve");
        assert_eq!(profile.properties.len(), 1);
        assert_eq!(profile.properties[0].name, "textures");
    }
}
