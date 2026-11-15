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

pub(crate) fn different_networks<'a>(
    current: &'a ServerConfig,
    target: &'a ServerConfig,
) -> Option<(&'a str, &'a str)> {
    match (&current.network, &target.network) {
        (Some(a), Some(b)) if a != b => Some((a, b)),
        _ => None,
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
    fn only_two_named_networks_can_differ() {
        let unset = config_with_network("lobby", None);
        let alice = config_with_network("lobby", Some("alice-net"));
        let alice_too = config_with_network("survival", Some("alice-net"));
        let bob = config_with_network("survival", Some("bob-net"));

        assert_eq!(different_networks(&alice, &alice_too), None);
        assert_eq!(different_networks(&unset, &bob), None);
        assert_eq!(different_networks(&alice, &unset), None);
        assert_eq!(different_networks(&unset, &unset), None);
        assert_eq!(
            different_networks(&alice, &bob),
            Some(("alice-net", "bob-net"))
        );
    }
}
