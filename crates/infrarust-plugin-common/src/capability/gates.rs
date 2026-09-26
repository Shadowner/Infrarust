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

pub const SUBSCRIBE_GATES: &[(&str, Capability)] = &[
    ("chat-message", Capability::ChatIntercept),
    ("command-execute", Capability::ChatIntercept),
    ("plugin-message", Capability::PluginMessaging),
    ("raw-packet", Capability::RawPacket),
];

#[must_use]
pub fn required(interface: &str, function: &str) -> &'static [Capability] {
    let entry = |wanted: &str| {
        GATES
            .iter()
            .find(|(gated, name, _)| *gated == interface && *name == wanted)
    };
    entry(function)
        .or_else(|| entry("*"))
        .map_or(&[], |(_, _, capabilities)| capabilities)
}

#[must_use]
pub fn subscribe_gate(kind: &str) -> Option<Capability> {
    SUBSCRIBE_GATES
        .iter()
        .find(|(gated, _)| *gated == kind)
        .map(|(_, capability)| *capability)
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
