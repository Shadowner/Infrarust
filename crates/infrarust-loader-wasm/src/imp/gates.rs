use infrarust_api::permissions::{Capability, CapabilitySet};
use wasmtime::Engine;
use wasmtime::component::Component;
use wasmtime::component::types::ComponentItem;

use crate::error::WasmLoaderError;

const HOST_PACKAGE: &str = "infrarust:plugin/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MissingGrant {
    pub(crate) interface: String,
    pub(crate) capability: Capability,
    pub(crate) functions: Vec<String>,
}

pub(crate) fn gate(interface: &str, function: &str) -> Option<Capability> {
    match interface {
        "event-bus" if function == "subscribe-packets" => Some(Capability::RawPacket),
        "event-bus" => Some(Capability::EventBus),
        "players" => Some(player_gate(function)),
        "server-manager" => Some(Capability::ServerManage),
        "ban-service" => Some(Capability::Ban),
        "config-service" if function == "write-proxy-config-document" => {
            Some(Capability::ConfigWrite)
        }
        "config-service" => Some(Capability::ConfigRead),
        "load-balancer" if matches!(function, "set-drained" | "reset-backend") => {
            Some(Capability::ServerManage)
        }
        "load-balancer" => Some(Capability::ConfigRead),
        "messaging" => Some(Capability::PluginMessaging),
        "command-manager" => Some(Capability::Command),
        "scheduler" => Some(Capability::Scheduler),
        "codec-registry" => Some(Capability::CodecFilter),
        "limbo" if function == "register-limbo-handler" => Some(Capability::Limbo),
        _ => None,
    }
}

fn player_gate(function: &str) -> Capability {
    match function {
        "send-message"
        | "send-title"
        | "send-action-bar"
        | "switch-server"
        | "disconnect"
        | "connect"
        | "set-player-list-header-footer"
        | "clear-title"
        | "show-boss-bar"
        | "update-boss-bar"
        | "hide-boss-bar"
        | "send-resource-pack"
        | "remove-resource-pack"
        | "transfer"
        | "store-cookie"
        | "request-cookie"
        | "refresh-permissions" => Capability::PlayerWrite,
        "send-packet" => Capability::RawPacket,
        _ => Capability::PlayerRead,
    }
}

fn host_interface(import: &str) -> Option<&str> {
    let path = import.strip_prefix(HOST_PACKAGE)?;
    Some(path.split_once('@').map_or(path, |(name, _)| name))
}

pub(crate) fn missing_grants(
    engine: &Engine,
    component: &Component,
    granted: &CapabilitySet,
) -> Vec<MissingGrant> {
    let ty = component.component_type();
    let mut missing: Vec<MissingGrant> = Vec::new();
    for (import, item) in ty.imports(engine) {
        let Some(interface) = host_interface(import) else {
            continue;
        };
        let ComponentItem::ComponentInstance(instance) = item else {
            continue;
        };
        for (function, item) in instance.exports(engine) {
            if !matches!(item, ComponentItem::ComponentFunc(_)) {
                continue;
            }
            let Some(capability) = gate(interface, function).filter(|cap| !granted.has(*cap))
            else {
                continue;
            };
            match missing
                .iter_mut()
                .find(|m| m.interface == interface && m.capability == capability)
            {
                Some(entry) => entry.functions.push(function.to_owned()),
                None => missing.push(MissingGrant {
                    interface: interface.to_owned(),
                    capability,
                    functions: vec![function.to_owned()],
                }),
            }
        }
    }
    missing
}

pub(crate) fn check_imports(
    engine: &Engine,
    component: &Component,
    plugin_id: &str,
    granted: &CapabilitySet,
    strict: bool,
) -> Result<(), WasmLoaderError> {
    let missing = missing_grants(engine, component, granted);
    if strict && !missing.is_empty() {
        let needs: Vec<String> = missing
            .iter()
            .map(|m| {
                format!(
                    "{HOST_PACKAGE}{} needs `{}` ({})",
                    m.interface,
                    m.capability.to_kebab(),
                    m.functions.join(", ")
                )
            })
            .collect();
        return Err(WasmLoaderError::CapabilityDenied {
            plugin_id: plugin_id.to_owned(),
            reason: format!(
                "{}; refused because strict_capabilities = true",
                needs.join("; ")
            ),
        });
    }
    for m in &missing {
        let capability = m.capability.to_kebab();
        tracing::warn!(
            plugin = %plugin_id,
            interface = %m.interface,
            capability,
            functions = %m.functions.join(","),
            "plugin {plugin_id} imports {} but lacks the `{capability}` capability; calls will be refused",
            m.interface
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        let config: infrarust_config::ProxyConfig = toml::from_str("").unwrap();
        crate::engine::build_engine(&config).unwrap()
    }

    const IMPORTS: &str = r#"
        (component
            (import "infrarust:plugin/ban-service@0.3.0" (instance
                (export "get" (func))
                (export "list" (func))
            ))
            (import "infrarust:plugin/limbo@0.3.0" (instance
                (export "limbo-session" (type (sub resource)))
            ))
            (import "infrarust:plugin/players@0.3.0" (instance
                (export "list" (func))
                (export "send-message" (func))
                (export "send-packet" (func))
            ))
            (import "infrarust:plugin/text@0.3.0" (instance
                (export "to-json" (func))
            ))
            (import "infrarust:plugin/log@0.3.0" (instance
                (export "info" (func))
            ))
            (import "wasi:random/insecure-seed@0.2.0" (instance
                (export "insecure-seed" (func))
            ))
        )
    "#;

    fn grants(missing: &[MissingGrant]) -> Vec<(String, Capability, Vec<String>)> {
        missing
            .iter()
            .map(|m| (m.interface.clone(), m.capability, m.functions.clone()))
            .collect()
    }

    #[test]
    fn reports_each_imported_function_whose_capability_is_missing() {
        let engine = engine();
        let component = Component::new(&engine, IMPORTS).unwrap();
        let missing = missing_grants(&engine, &component, &CapabilitySet::default());
        assert_eq!(
            grants(&missing),
            [
                (
                    "ban-service".to_string(),
                    Capability::Ban,
                    vec!["get".to_string(), "list".to_string()]
                ),
                (
                    "players".to_string(),
                    Capability::PlayerRead,
                    vec!["list".to_string()]
                ),
                (
                    "players".to_string(),
                    Capability::PlayerWrite,
                    vec!["send-message".to_string()]
                ),
                (
                    "players".to_string(),
                    Capability::RawPacket,
                    vec!["send-packet".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn baseline_grants_leave_only_the_opt_ins_in_the_report() {
        let engine = engine();
        let component = Component::new(&engine, IMPORTS).unwrap();
        let missing = missing_grants(&engine, &component, &CapabilitySet::baseline());
        let capabilities: Vec<Capability> = missing.iter().map(|m| m.capability).collect();
        assert_eq!(capabilities, [Capability::Ban, Capability::RawPacket]);

        let everything = CapabilitySet::native_trusted();
        assert!(missing_grants(&engine, &component, &everything).is_empty());
    }

    #[test]
    fn strict_mode_turns_the_report_into_a_load_error() {
        let engine = engine();
        let component = Component::new(&engine, IMPORTS).unwrap();
        let baseline = CapabilitySet::baseline();

        assert!(check_imports(&engine, &component, "p", &baseline, false).is_ok());
        let err = check_imports(&engine, &component, "p", &baseline, true)
            .expect_err("strict mode refuses an ungranted import")
            .to_string();
        assert!(
            err.contains("infrarust:plugin/ban-service needs `ban`"),
            "{err}"
        );
        assert!(
            err.contains("infrarust:plugin/players needs `raw-packet`"),
            "{err}"
        );
        assert!(err.contains("strict_capabilities"), "{err}");

        let granted = baseline.with(Capability::Ban).with(Capability::RawPacket);
        assert!(check_imports(&engine, &component, "p", &granted, true).is_ok());
    }

    #[test]
    fn every_function_of_the_contract_has_a_gate_decision() {
        assert_eq!(gate("limbo", "[method]limbo-session.send-message"), None);
        assert_eq!(
            gate("limbo", "register-limbo-handler"),
            Some(Capability::Limbo)
        );
        assert_eq!(gate("log", "info"), None);
        assert_eq!(gate("text", "parse-json"), None);
        assert_eq!(gate("types", "anything"), None);
        assert_eq!(gate("events", "anything"), None);
        assert_eq!(gate("players", "get"), Some(Capability::PlayerRead));
        assert_eq!(
            gate("players", "has-permission"),
            Some(Capability::PlayerRead)
        );
        assert_eq!(gate("players", "disconnect"), Some(Capability::PlayerWrite));
        assert_eq!(gate("players", "send-packet"), Some(Capability::RawPacket));
        assert_eq!(gate("event-bus", "unsubscribe"), Some(Capability::EventBus));
        assert_eq!(gate("event-bus", "fire-named"), Some(Capability::EventBus));
        assert_eq!(
            gate("event-bus", "subscribe-packets"),
            Some(Capability::RawPacket)
        );
        assert_eq!(gate("players", "connect"), Some(Capability::PlayerWrite));
        assert_eq!(
            gate("players", "request-cookie"),
            Some(Capability::PlayerWrite)
        );
        assert_eq!(
            gate("messaging", "send-to-server"),
            Some(Capability::PluginMessaging)
        );
        assert_eq!(
            gate("load-balancer", "backends"),
            Some(Capability::ConfigRead)
        );
        assert_eq!(
            gate("load-balancer", "set-drained"),
            Some(Capability::ServerManage)
        );
        assert_eq!(
            gate("config-service", "write-proxy-config-document"),
            Some(Capability::ConfigWrite)
        );
        assert_eq!(
            gate("config-service", "list-server-sources"),
            Some(Capability::ConfigRead)
        );
        assert_eq!(gate("proxy-info", "granted-capabilities"), None);
        assert_eq!(gate("plugin-registry", "list"), None);
    }
}
