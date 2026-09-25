//! Game profile types for Mojang authentication.

use infrarust_config::OfflineUuidPolicy;
use md5::{Digest, Md5};
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
    ///
    /// # Errors
    /// Returns `CoreError::Auth` if the id string is not a valid UUID.
    pub fn uuid(&self) -> Result<Uuid, CoreError> {
        Uuid::parse_str(&self.id).map_err(|e| CoreError::Auth(format!("invalid uuid: {e}")))
    }
}

pub fn offline_uuid(username: &str) -> Uuid {
    let digest = Md5::digest(format!("OfflinePlayer:{username}").as_bytes());
    uuid::Builder::from_md5_bytes(digest.into()).into_uuid()
}

pub fn offline_profile_uuid(
    policy: OfflineUuidPolicy,
    username: &str,
    claimed: Option<Uuid>,
) -> Uuid {
    match (policy, claimed) {
        (OfflineUuidPolicy::Client, Some(claimed)) => claimed,
        _ => offline_uuid(username),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn offline_profiles_ignore_the_claimed_uuid_by_default() {
        let claimed = Uuid::from_u128(7);
        assert_eq!(
            offline_profile_uuid(OfflineUuidPolicy::Offline, "Steve", Some(claimed)),
            offline_uuid("Steve")
        );
        assert_eq!(
            offline_profile_uuid(OfflineUuidPolicy::Offline, "Steve", None),
            offline_uuid("Steve")
        );
    }

    #[test]
    fn the_client_policy_trusts_a_claimed_uuid_only_when_one_is_sent() {
        let claimed = Uuid::from_u128(7);
        assert_eq!(
            offline_profile_uuid(OfflineUuidPolicy::Client, "Steve", Some(claimed)),
            claimed
        );
        assert_eq!(
            offline_profile_uuid(OfflineUuidPolicy::Client, "Steve", None),
            offline_uuid("Steve")
        );
    }

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
    fn offline_uuid_matches_java_name_uuid_from_bytes() {
        for (name, expected) in [
            ("Notch", "b50ad385-829d-3141-a216-7e7d7539ba7f"),
            ("jeb_", "a762f560-4fce-3236-812a-b80efff0b62b"),
            ("Été", "31870ee5-f805-3d77-87f7-0fefd0b0dc3b"),
            ("Steve", "5627dd98-e6be-3c21-b8a8-e92344183641"),
        ] {
            assert_eq!(offline_uuid(name).to_string(), expected, "{name}");
        }
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
