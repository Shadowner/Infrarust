//! Ban system configuration.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BanConfig {
    #[serde(default)]
    pub provider: BanProviderSelection,

    /// Path to the JSON bans file.
    #[serde(default = "defaults::ban_file")]
    pub file: std::path::PathBuf,

    /// Automatic purge interval for expired bans.
    #[serde(default = "defaults::ban_purge_interval")]
    #[serde(with = "humantime_serde")]
    pub purge_interval: Duration,

    /// Enables the audit log (tracks ban/unban operations).
    #[serde(default = "defaults::ban_audit_log")]
    pub enable_audit_log: bool,
}

impl Default for BanConfig {
    fn default() -> Self {
        Self {
            provider: BanProviderSelection::default(),
            file: defaults::ban_file(),
            purge_interval: defaults::ban_purge_interval(),
            enable_audit_log: defaults::ban_audit_log(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum BanProviderSelection {
    #[default]
    Builtin,
    Disabled,
    Plugin(String),
}

impl BanProviderSelection {
    pub const BUILTIN: &'static str = "builtin";
    pub const DISABLED: &'static str = "none";

    pub fn plugin_id(&self) -> Option<&str> {
        match self {
            Self::Plugin(id) => Some(id),
            _ => None,
        }
    }
}

impl fmt::Display for BanProviderSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => f.write_str(Self::BUILTIN),
            Self::Disabled => f.write_str(Self::DISABLED),
            Self::Plugin(id) => f.write_str(id),
        }
    }
}

impl TryFrom<String> for BanProviderSelection {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        match trimmed {
            "" => Err("ban provider must be \"builtin\", \"none\" or a plugin id".to_string()),
            Self::BUILTIN => Ok(Self::Builtin),
            Self::DISABLED => Ok(Self::Disabled),
            id => Ok(Self::Plugin(id.to_string())),
        }
    }
}

impl From<BanProviderSelection> for String {
    fn from(value: BanProviderSelection) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_missing_provider_keeps_the_builtin_one() {
        let config: BanConfig = toml::from_str("file = \"bans.json\"").unwrap();
        assert_eq!(config.provider, BanProviderSelection::Builtin);
    }

    #[test]
    fn the_provider_names_builtin_none_or_a_plugin() {
        for (text, expected) in [
            ("builtin", BanProviderSelection::Builtin),
            ("none", BanProviderSelection::Disabled),
            (
                "libertybans",
                BanProviderSelection::Plugin("libertybans".into()),
            ),
        ] {
            let config: BanConfig = toml::from_str(&format!("provider = \"{text}\"")).unwrap();
            assert_eq!(config.provider, expected);
            let written = toml::to_string(&config).unwrap();
            assert!(
                written.contains(&format!("provider = \"{text}\"")),
                "{written}"
            );
        }
        assert!(toml::from_str::<BanConfig>("provider = \"\"").is_err());
    }
}
