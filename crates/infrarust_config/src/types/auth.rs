use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default = "defaults::session_url")]
    pub session_url: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_url: defaults::session_url(),
        }
    }
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
    fn omitting_the_url_keeps_the_mojang_default() {
        let config: AuthConfig = toml::from_str("").expect("empty auth config");
        assert_eq!(config.session_url, defaults::session_url());
    }
}
