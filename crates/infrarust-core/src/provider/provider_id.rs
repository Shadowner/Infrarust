use std::fmt;
use std::str::FromStr;

use crate::error::CoreError;

const PLUGIN_SOURCE_PREFIX: &str = "plugin:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProviderTrust {
    Discovered,
    Configured,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProviderSource {
    File,
    Docker,
    Plugin { plugin: String, kind: String },
}

impl ProviderSource {
    pub const fn trust(&self) -> ProviderTrust {
        match self {
            Self::File | Self::Plugin { .. } => ProviderTrust::Configured,
            Self::Docker => ProviderTrust::Discovered,
        }
    }
}

impl fmt::Display for ProviderSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File => f.write_str("file"),
            Self::Docker => f.write_str("docker"),
            Self::Plugin { plugin, kind } => write!(f, "{PLUGIN_SOURCE_PREFIX}{plugin}:{kind}"),
        }
    }
}

impl FromStr for ProviderSource {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "file" => Ok(Self::File),
            "docker" => Ok(Self::Docker),
            _ => s
                .strip_prefix(PLUGIN_SOURCE_PREFIX)
                .and_then(|owned| owned.split_once(':'))
                .map(|(plugin, kind)| Self::Plugin {
                    plugin: plugin.to_string(),
                    kind: kind.to_string(),
                })
                .ok_or_else(|| CoreError::InvalidProviderId(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderId {
    pub source: ProviderSource,
    pub unique_id: String,
}

impl ProviderId {
    pub fn file(filename: impl Into<String>) -> Self {
        Self {
            source: ProviderSource::File,
            unique_id: filename.into(),
        }
    }

    pub fn docker(container_name: impl Into<String>) -> Self {
        Self {
            source: ProviderSource::Docker,
            unique_id: container_name.into(),
        }
    }

    pub fn plugin(plugin_id: &str, kind: &str, document: impl Into<String>) -> Self {
        Self {
            source: ProviderSource::Plugin {
                plugin: plugin_id.to_string(),
                kind: kind.to_string(),
            },
            unique_id: document.into(),
        }
    }

    pub fn plugin_owner(&self) -> Option<&str> {
        match &self.source {
            ProviderSource::Plugin { plugin, .. } => Some(plugin),
            ProviderSource::File | ProviderSource::Docker => None,
        }
    }

    pub const fn trust(&self) -> ProviderTrust {
        self.source.trust()
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.source, self.unique_id)
    }
}

impl FromStr for ProviderId {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (source, unique_id) = s
            .split_once('@')
            .ok_or_else(|| CoreError::InvalidProviderId(s.to_string()))?;
        Ok(Self {
            source: source.parse()?,
            unique_id: unique_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn plugin_ids_name_their_owner() {
        let id = ProviderId::plugin("k8s", "api", "lobby");
        assert_eq!(id.to_string(), "plugin:k8s:api@lobby");
        assert_eq!(id.plugin_owner(), Some("k8s"));
        assert_eq!(ProviderId::file("a.toml").plugin_owner(), None);
    }

    #[test]
    fn only_docker_configs_are_discovered() {
        assert_eq!(
            ProviderId::file("a.toml").trust(),
            ProviderTrust::Configured
        );
        assert_eq!(
            ProviderId::plugin("k8s", "api", "lobby").trust(),
            ProviderTrust::Configured
        );
        assert_eq!(ProviderId::docker("mc").trust(), ProviderTrust::Discovered);
    }

    #[test]
    fn an_unknown_source_does_not_parse() {
        assert!("api@lobby".parse::<ProviderId>().is_err());
        assert!("plugin:bare@lobby".parse::<ProviderId>().is_err());
    }
}
