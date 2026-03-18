//! Network validation for server switching.
//!
//! Ensures that switches are only allowed between servers in the same network.

use infrarust_config::ServerConfig;

/// Error when a switch is not allowed due to network constraints.
#[derive(Debug, thiserror::Error)]
pub enum SwitchValidationError {
    #[error("current server '{0}' is not part of any network — switch not allowed")]
    SourceNotInNetwork(String),

    #[error("target server '{0}' is not part of any network — switch not allowed")]
    TargetNotInNetwork(String),

    #[error("servers are in different networks: '{current}' vs '{target}'")]
    DifferentNetworks { current: String, target: String },
}

/// Validates that a switch between two servers is allowed based on network membership.
///
/// Rules:
/// - Both servers must have a `network` set (non-`None`)
/// - Both must be in the same network
pub fn validate_switch_allowed(
    current: &ServerConfig,
    target: &ServerConfig,
) -> Result<(), SwitchValidationError> {
    match (&current.network, &target.network) {
        (Some(a), Some(b)) if a == b => Ok(()),
        (None, _) => Err(SwitchValidationError::SourceNotInNetwork(
            current.effective_id(),
        )),
        (_, None) => Err(SwitchValidationError::TargetNotInNetwork(
            target.effective_id(),
        )),
        (Some(a), Some(b)) => Err(SwitchValidationError::DifferentNetworks {
            current: a.clone(),
            target: b.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn config_with_network(name: &str, network: Option<&str>) -> ServerConfig {
        toml::from_str(&format!(
            "domains = [\"test.example.com\"]\naddresses = [\"127.0.0.1:25565\"]\nname = \"{name}\"{network_line}",
            network_line = network.map(|n| format!("\nnetwork = \"{n}\"")).unwrap_or_default()
        ))
        .unwrap()
    }

    #[test]
    fn test_switch_same_network_allowed() {
        let a = config_with_network("lobby", Some("my-network"));
        let b = config_with_network("survival", Some("my-network"));
        assert!(validate_switch_allowed(&a, &b).is_ok());
    }

    #[test]
    fn test_switch_source_no_network() {
        let a = config_with_network("lobby", None);
        let b = config_with_network("survival", Some("my-network"));
        let err = validate_switch_allowed(&a, &b).unwrap_err();
        assert!(matches!(err, SwitchValidationError::SourceNotInNetwork(_)));
    }

    #[test]
    fn test_switch_target_no_network() {
        let a = config_with_network("lobby", Some("my-network"));
        let b = config_with_network("survival", None);
        let err = validate_switch_allowed(&a, &b).unwrap_err();
        assert!(matches!(err, SwitchValidationError::TargetNotInNetwork(_)));
    }

    #[test]
    fn test_switch_different_networks() {
        let a = config_with_network("lobby", Some("alice-net"));
        let b = config_with_network("survival", Some("bob-net"));
        let err = validate_switch_allowed(&a, &b).unwrap_err();
        assert!(matches!(
            err,
            SwitchValidationError::DifferentNetworks { .. }
        ));
    }

    #[test]
    fn test_switch_both_no_network() {
        let a = config_with_network("lobby", None);
        let b = config_with_network("survival", None);
        let err = validate_switch_allowed(&a, &b).unwrap_err();
        assert!(matches!(err, SwitchValidationError::SourceNotInNetwork(_)));
    }
}
