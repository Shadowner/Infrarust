use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionsConfig {
    pub provider: PermissionProviderSelection,

    pub admins: Vec<String>,

    /// Subcommands of `/ir` accessible to all players (not just admins).
    pub player_commands: Vec<String>,

    pub trust_offline_admins: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum PermissionProviderSelection {
    #[default]
    Builtin,
    Plugin(String),
}

impl PermissionProviderSelection {
    pub const BUILTIN: &'static str = "builtin";

    pub fn plugin_id(&self) -> Option<&str> {
        match self {
            Self::Plugin(id) => Some(id),
            Self::Builtin => None,
        }
    }
}

impl fmt::Display for PermissionProviderSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => f.write_str(Self::BUILTIN),
            Self::Plugin(id) => f.write_str(id),
        }
    }
}

impl TryFrom<String> for PermissionProviderSelection {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.trim() {
            "" => Err("permission provider must be \"builtin\" or a plugin id".to_string()),
            Self::BUILTIN => Ok(Self::Builtin),
            id => Ok(Self::Plugin(id.to_string())),
        }
    }
}

impl From<PermissionProviderSelection> for String {
    fn from(value: PermissionProviderSelection) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn an_empty_section_keeps_todays_behaviour() {
        let config: PermissionsConfig = toml::from_str("").unwrap();
        assert_eq!(config.provider, PermissionProviderSelection::Builtin);
        assert!(!config.trust_offline_admins);
        assert!(config.admins.is_empty());
    }

    #[test]
    fn the_provider_names_builtin_or_a_plugin() {
        for (text, expected) in [
            ("builtin", PermissionProviderSelection::Builtin),
            (
                "luckperms",
                PermissionProviderSelection::Plugin("luckperms".into()),
            ),
        ] {
            let config: PermissionsConfig =
                toml::from_str(&format!("provider = \"{text}\"")).unwrap();
            assert_eq!(config.provider, expected);
            let written = toml::to_string(&config).unwrap();
            assert!(
                written.contains(&format!("provider = \"{text}\"")),
                "{written}"
            );
        }
        assert!(toml::from_str::<PermissionsConfig>("provider = \" \"").is_err());
    }

    #[test]
    fn trust_offline_admins_is_read() {
        let config: PermissionsConfig = toml::from_str("trust_offline_admins = true").unwrap();
        assert!(config.trust_offline_admins);
    }

    #[test]
    fn every_documented_key_loads() {
        let config: PermissionsConfig = toml::from_str(
            "provider = \"luckperms\"\n\
             admins = [\"Notch\", \"069a79f4-44e9-4726-a5be-fca90e38aaf5\"]\n\
             player_commands = [\"help\", \"list\"]\n\
             trust_offline_admins = true\n",
        )
        .unwrap();
        assert_eq!(
            config.provider,
            PermissionProviderSelection::Plugin("luckperms".into())
        );
        assert_eq!(config.admins.len(), 2);
        assert_eq!(config.player_commands, ["help", "list"]);
        assert!(config.trust_offline_admins);
    }

    #[test]
    fn a_misspelled_key_is_refused() {
        let error = toml::from_str::<PermissionsConfig>("provder = \"luckperms\"")
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field `provder`"), "{error}");
        assert!(error.contains("`provider`"), "{error}");
    }

    #[test]
    fn a_misspelled_key_stops_the_proxy_config_from_loading() {
        let error = toml::from_str::<crate::ProxyConfig>("[permissions]\nadmin = [\"Notch\"]\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field `admin`"), "{error}");
    }
}
