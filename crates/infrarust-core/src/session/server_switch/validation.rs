//! Network validation for server switching.
//!
//! Ensures that switches are only allowed between servers in the same network.

use infrarust_api::events::connection::ConnectCause;
use infrarust_config::ServerConfig;

use crate::player::SessionKind;

pub(crate) const UNVERIFIED_REFUSED: &str = "This server requires a verified Minecraft account";

pub(crate) const fn admits_session(kind: SessionKind, target: &ServerConfig) -> bool {
    kind.online_mode() || !target.proxy_mode.requires_verified_player()
}

pub(crate) fn backend_scope_contains(origin: &ServerConfig, target: &ServerConfig) -> bool {
    match (&origin.network, &target.network) {
        (Some(a), Some(b)) => a == b,
        (None, None) => origin.effective_id() == target.effective_id(),
        (Some(_), None) | (None, Some(_)) => false,
    }
}

pub(crate) fn switch_in_scope(
    cause: ConnectCause,
    current: &ServerConfig,
    target: &ServerConfig,
) -> bool {
    match cause {
        ConnectCause::Initial
        | ConnectCause::Switch
        | ConnectCause::LimboExit
        | ConnectCause::KickRedirect => true,
        _ => backend_scope_contains(current, target),
    }
}

/// Error when a switch is not allowed due to network constraints.
#[derive(Debug, thiserror::Error)]
pub enum SwitchValidationError {
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
        // Same network: always allowed.
        (Some(a), Some(b)) if a == b => Ok(()),
        // Different networks: blocked.
        (Some(a), Some(b)) => Err(SwitchValidationError::DifferentNetworks {
            current: a.clone(),
            target: b.clone(),
        }),
        // Either has no network: allowed (no network = no restriction).
        // TODO: In future, only allow this when the source is a limbo or virtual backend,
        // not for arbitrary servers without a network declaration.
        _ => Ok(()),
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
    fn a_server_without_a_network_is_only_its_own_scope() {
        let lobby = config_with_network("lobby", None);
        let game = config_with_network("game", None);
        let main = config_with_network("main", Some("n"));
        let same = config_with_network("same", Some("n"));
        let other = config_with_network("other", Some("o"));

        assert!(backend_scope_contains(&lobby, &lobby));
        assert!(!backend_scope_contains(&lobby, &game));
        assert!(!backend_scope_contains(&lobby, &main));
        assert!(!backend_scope_contains(&main, &lobby));
        assert!(backend_scope_contains(&main, &same));
        assert!(!backend_scope_contains(&main, &other));
    }

    #[test]
    fn only_backend_requests_are_held_to_the_scope() {
        let lobby = config_with_network("lobby", None);
        let game = config_with_network("game", None);

        assert!(!switch_in_scope(ConnectCause::PluginMessage, &lobby, &game));
        assert!(switch_in_scope(ConnectCause::PluginMessage, &lobby, &lobby));
        for cause in [
            ConnectCause::Initial,
            ConnectCause::Switch,
            ConnectCause::LimboExit,
            ConnectCause::KickRedirect,
        ] {
            assert!(switch_in_scope(cause, &lobby, &game), "{cause:?}");
        }
    }

    #[test]
    fn test_switch_same_network_allowed() {
        let a = config_with_network("lobby", Some("my-network"));
        let b = config_with_network("survival", Some("my-network"));
        assert!(validate_switch_allowed(&a, &b).is_ok());
    }

    #[test]
    fn test_switch_source_no_network_allowed() {
        let a = config_with_network("lobby", None);
        let b = config_with_network("survival", Some("my-network"));
        assert!(validate_switch_allowed(&a, &b).is_ok());
    }

    #[test]
    fn test_switch_target_no_network_allowed() {
        let a = config_with_network("lobby", Some("my-network"));
        let b = config_with_network("survival", None);
        assert!(validate_switch_allowed(&a, &b).is_ok());
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
    fn test_switch_both_no_network_allowed() {
        let a = config_with_network("lobby", None);
        let b = config_with_network("survival", None);
        assert!(validate_switch_allowed(&a, &b).is_ok());
    }
}
