use super::Capability;

pub const GATES: &[(&str, &str, &[Capability])] = &[
    (
        "event-bus",
        "subscribe-packets",
        &[Capability::EventBus, Capability::RawPacket],
    ),
    ("event-bus", "*", &[Capability::EventBus]),
    ("players", "send-message", &[Capability::PlayerWrite]),
    ("players", "send-title", &[Capability::PlayerWrite]),
    ("players", "send-action-bar", &[Capability::PlayerWrite]),
    ("players", "switch-server", &[Capability::PlayerWrite]),
    ("players", "disconnect", &[Capability::PlayerWrite]),
    ("players", "connect", &[Capability::PlayerWrite]),
    (
        "players",
        "set-player-list-header-footer",
        &[Capability::PlayerWrite],
    ),
    ("players", "clear-title", &[Capability::PlayerWrite]),
    ("players", "show-boss-bar", &[Capability::PlayerWrite]),
    ("players", "update-boss-bar", &[Capability::PlayerWrite]),
    ("players", "hide-boss-bar", &[Capability::PlayerWrite]),
    ("players", "send-resource-pack", &[Capability::PlayerWrite]),
    (
        "players",
        "remove-resource-pack",
        &[Capability::PlayerWrite],
    ),
    ("players", "transfer", &[Capability::PlayerWrite]),
    ("players", "store-cookie", &[Capability::PlayerWrite]),
    ("players", "request-cookie", &[Capability::PlayerWrite]),
    ("players", "refresh-permissions", &[Capability::PlayerWrite]),
    ("players", "send-packet", &[Capability::RawPacket]),
    ("players", "*", &[Capability::PlayerRead]),
    ("server-manager", "*", &[Capability::ServerManage]),
    ("ban-service", "*", &[Capability::Ban]),
    (
        "config-service",
        "write-proxy-config-document",
        &[Capability::ConfigWrite],
    ),
    ("config-service", "*", &[Capability::ConfigRead]),
    ("load-balancer", "set-drained", &[Capability::ServerManage]),
    (
        "load-balancer",
        "reset-backend",
        &[Capability::ServerManage],
    ),
    ("load-balancer", "*", &[Capability::ConfigRead]),
    ("messaging", "*", &[Capability::PluginMessaging]),
    ("command-manager", "*", &[Capability::Command]),
    ("scheduler", "*", &[Capability::Scheduler]),
    ("codec-registry", "*", &[Capability::CodecFilter]),
    ("limbo", "register-limbo-handler", &[Capability::Limbo]),
    ("permissions", "*", &[Capability::PermissionProvider]),
    (
        "providers",
        "register-ban-provider",
        &[Capability::BanProvider],
    ),
    ("providers", "*", &[Capability::PermissionProvider]),
];

pub const UNGATED: &[(&str, &str)] = &[
    ("limbo", "[method]limbo-session.player-id"),
    ("limbo", "[method]limbo-session.profile"),
    ("limbo", "[method]limbo-session.entry-context"),
    ("limbo", "[method]limbo-session.send-message"),
    ("limbo", "[method]limbo-session.send-title"),
    ("limbo", "[method]limbo-session.send-action-bar"),
    ("limbo", "[method]limbo-session.complete"),
    ("limbo", "[method]limbo-session.acquire-handle"),
    ("limbo", "[method]limbo-session-handle.player-id"),
    ("limbo", "[method]limbo-session-handle.send-message"),
    ("limbo", "[method]limbo-session-handle.send-title"),
    ("limbo", "[method]limbo-session-handle.send-action-bar"),
    ("limbo", "[method]limbo-session-handle.complete"),
    ("limbo", "[method]limbo-session-handle.cancelled"),
    ("events", "ping-description"),
    ("events", "ping-favicon"),
    ("events", "ping-player-sample"),
    ("log", "max-level"),
    ("log", "trace"),
    ("log", "debug"),
    ("log", "info"),
    ("log", "warn"),
    ("log", "error"),
    ("text", "parse-json"),
    ("text", "parse-legacy"),
    ("text", "to-json"),
    ("text", "to-plain"),
    ("proxy-info", "details"),
    ("proxy-info", "granted-capabilities"),
    ("plugin-registry", "list"),
    ("plugin-registry", "get"),
];

pub const SUBSCRIBE_GATES: &[(&str, Capability)] = &[
    ("chat-message", Capability::ChatIntercept),
    ("command-execute", Capability::ChatIntercept),
    ("plugin-message", Capability::PluginMessaging),
    ("raw-packet", Capability::RawPacket),
];

const fn same(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut at = 0;
    while at < left.len() {
        if left[at] != right[at] {
            return false;
        }
        at += 1;
    }
    true
}

const fn entry(interface: &str, function: &str) -> Option<&'static [Capability]> {
    let mut at = 0;
    while at < GATES.len() {
        let (gated, name, capabilities) = GATES[at];
        if same(gated, interface) && same(name, function) {
            return Some(capabilities);
        }
        at += 1;
    }
    None
}

const fn ungated(interface: &str, function: &str) -> bool {
    let mut at = 0;
    while at < UNGATED.len() {
        let (open, name) = UNGATED[at];
        if same(open, interface) && same(name, function) {
            return true;
        }
        at += 1;
    }
    false
}

#[must_use]
pub const fn is_known(interface: &str, function: &str) -> bool {
    entry(interface, function).is_some()
        || entry(interface, "*").is_some()
        || ungated(interface, function)
}

#[must_use]
pub const fn required(interface: &str, function: &str) -> &'static [Capability] {
    match entry(interface, function) {
        Some(capabilities) => capabilities,
        None => match entry(interface, "*") {
            Some(capabilities) => capabilities,
            None => &[],
        },
    }
}

#[must_use]
pub const fn subscribe_gate(kind: &str) -> Option<Capability> {
    let mut at = 0;
    while at < SUBSCRIBE_GATES.len() {
        let (gated, capability) = SUBSCRIBE_GATES[at];
        if same(gated, kind) {
            return Some(capability);
        }
        at += 1;
    }
    None
}

pub fn gated_interfaces() -> impl Iterator<Item = &'static str> {
    GATES
        .iter()
        .enumerate()
        .filter(|(at, (interface, _, _))| GATES[..*at].iter().all(|(seen, _, _)| seen != interface))
        .map(|(_, (interface, _, _))| *interface)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn an_exact_entry_wins_over_the_wildcard_of_its_interface() {
        assert_eq!(
            required("event-bus", "subscribe-packets"),
            [Capability::EventBus, Capability::RawPacket]
        );
        assert_eq!(required("event-bus", "unsubscribe"), [Capability::EventBus]);
        assert_eq!(required("players", "send-packet"), [Capability::RawPacket]);
        assert_eq!(required("players", "disconnect"), [Capability::PlayerWrite]);
        assert_eq!(required("players", "get"), [Capability::PlayerRead]);
        assert_eq!(
            required("config-service", "write-proxy-config-document"),
            [Capability::ConfigWrite]
        );
        assert_eq!(
            required("config-service", "list-server-sources"),
            [Capability::ConfigRead]
        );
        assert_eq!(
            required("load-balancer", "set-drained"),
            [Capability::ServerManage]
        );
        assert_eq!(
            required("load-balancer", "backends"),
            [Capability::ConfigRead]
        );
        assert_eq!(
            required("providers", "register-ban-provider"),
            [Capability::BanProvider]
        );
        assert_eq!(
            required("providers", "register-permission-provider"),
            [Capability::PermissionProvider]
        );
    }

    #[test]
    fn ungated_functions_and_interfaces_need_nothing() {
        assert_eq!(
            required("limbo", "register-limbo-handler"),
            [Capability::Limbo]
        );
        assert!(required("limbo", "[method]limbo-session.send-message").is_empty());
        assert!(required("log", "info").is_empty());
        assert!(required("text", "parse-json").is_empty());
        assert!(required("proxy-info", "granted-capabilities").is_empty());
        assert!(required("plugin-registry", "list").is_empty());
        for (interface, function) in UNGATED {
            assert!(required(interface, function).is_empty());
        }
    }

    #[test]
    fn only_gated_or_listed_functions_are_known() {
        assert!(is_known("players", "send-message"));
        assert!(is_known("players", "get"));
        assert!(is_known("limbo", "register-limbo-handler"));
        assert!(is_known("limbo", "[method]limbo-session.send-message"));
        assert!(is_known("log", "info"));
        assert!(!is_known("player", "send-message"));
        assert!(!is_known("log", "inf"));
        assert!(!is_known("limbo", "register-limbo-handlers"));
        assert!(!is_known("new-privileged-interface", "do-it"));
    }

    #[test]
    fn every_wildcard_closes_its_interface() {
        for (at, (interface, function, _)) in GATES.iter().enumerate() {
            if *function != "*" {
                continue;
            }
            assert!(
                GATES[at + 1..]
                    .iter()
                    .all(|(later, _, _)| later != interface),
                "`{interface}` has entries after its wildcard"
            );
        }
        let interfaces: Vec<&str> = gated_interfaces().collect();
        let mut unique = interfaces.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(interfaces.len(), unique.len());
        assert_eq!(interfaces.len(), 13);
        assert!(interfaces.contains(&"limbo"));
    }

    #[test]
    fn the_table_resolves_at_compile_time_like_at_run_time() {
        const SEND_MESSAGE: &[Capability] = required("players", "send-message");
        const READ_PLAYERS: &[Capability] = required("players", "list");
        const LOG: &[Capability] = required("log", "info");
        const CHAT: Option<Capability> = subscribe_gate("chat-message");
        assert_eq!(SEND_MESSAGE, [Capability::PlayerWrite]);
        assert_eq!(READ_PLAYERS, [Capability::PlayerRead]);
        assert!(LOG.is_empty());
        assert_eq!(CHAT, Some(Capability::ChatIntercept));
        for (interface, function, capabilities) in GATES {
            if *function != "*" {
                assert_eq!(required(interface, function), *capabilities);
            }
        }
        assert_eq!(required("players", "send-messag"), [Capability::PlayerRead]);
        assert!(required("player", "send-message").is_empty());
    }

    #[test]
    fn subscriptions_are_gated_by_event_kind() {
        assert_eq!(
            subscribe_gate("chat-message"),
            Some(Capability::ChatIntercept)
        );
        assert_eq!(
            subscribe_gate("command-execute"),
            Some(Capability::ChatIntercept)
        );
        assert_eq!(
            subscribe_gate("plugin-message"),
            Some(Capability::PluginMessaging)
        );
        assert_eq!(subscribe_gate("raw-packet"), Some(Capability::RawPacket));
        assert_eq!(subscribe_gate("post-login"), None);
    }
}
