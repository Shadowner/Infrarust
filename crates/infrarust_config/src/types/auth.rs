use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default = "defaults::session_url")]
    pub session_url: String,

    #[serde(default)]
    pub offline_uuid: OfflineUuidPolicy,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_url: defaults::session_url(),
            offline_uuid: OfflineUuidPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfflineUuidPolicy {
    #[default]
    Offline,
    Client,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_mojang() {
        let config = AuthConfig::default();
        assert_eq!(
            config.session_url,
            "https://sessionserver.mojang.com/session/minecraft/hasJoined"
        );
    }

    #[test]
    fn parses_a_custom_session_url() {
        let config: AuthConfig =
            toml::from_str(r#"session_url = "http://127.0.0.1:8080/session/minecraft/hasJoined""#)
                .expect("valid auth config");
        assert_eq!(
            config.session_url,
            "http://127.0.0.1:8080/session/minecraft/hasJoined"
        );
    }

    #[test]
    fn offline_uuid_defaults_to_the_name_based_uuid() {
        let config: AuthConfig = toml::from_str("").expect("empty auth config");
        assert_eq!(config.offline_uuid, OfflineUuidPolicy::Offline);
    }

    #[test]
    fn offline_uuid_can_trust_the_client() {
        let config: AuthConfig =
            toml::from_str(r#"offline_uuid = "client""#).expect("valid auth config");
        assert_eq!(config.offline_uuid, OfflineUuidPolicy::Client);
        assert!(toml::from_str::<AuthConfig>(r#"offline_uuid = "random""#).is_err());
    }

    #[test]
    fn omitting_the_url_keeps_the_mojang_default() {
        let config: AuthConfig = toml::from_str("").expect("empty auth config");
        assert_eq!(config.session_url, defaults::session_url());
    }
}
