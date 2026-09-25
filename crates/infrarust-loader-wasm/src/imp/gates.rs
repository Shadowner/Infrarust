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
        "event-bus" => Some(Capability::EventBus),
        "player-registry" => Some(player_gate(function)),
        "server-manager" => Some(Capability::ServerManage),
        "ban-service" => Some(Capability::Ban),
        "config-service" => Some(Capability::ConfigRead),
        "command-manager" => Some(Capability::Command),
        "scheduler" => Some(Capability::Scheduler),
        "codec-registry" => Some(Capability::CodecFilter),
        "limbo" if function == "register-limbo-handler" => Some(Capability::Limbo),
        _ => None,
    }
}

fn player_gate(function: &str) -> Capability {
    match function {
        "[method]player.send-message"
        | "[method]player.send-title"
        | "[method]player.send-action-bar"
        | "[method]player.switch-server"
        | "[method]player.disconnect" => Capability::PlayerWrite,
        "[method]player.send-packet" => Capability::RawPacket,
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
            (import "infrarust:plugin/ban-service@0.2.3" (instance
                (export "is-banned" (func))
                (export "get-all-bans" (func))
            ))
            (import "infrarust:plugin/limbo@0.2.3" (instance
                (export "limbo-session" (type (sub resource)))
            ))
            (import "infrarust:plugin/player-registry@0.2.3" (instance
                (export "player" (type (sub resource)))
                (export "get-all-players" (func))
                (export "[method]player.send-message" (func (param "self" (borrow 0))))
                (export "[method]player.send-packet" (func (param "self" (borrow 0))))
            ))
            (import "infrarust:plugin/log@0.2.3" (instance
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
                    vec!["is-banned".to_string(), "get-all-bans".to_string()]
                ),
                (
                    "player-registry".to_string(),
                    Capability::PlayerRead,
                    vec!["get-all-players".to_string()]
                ),
                (
                    "player-registry".to_string(),
                    Capability::PlayerWrite,
                    vec!["[method]player.send-message".to_string()]
                ),
                (
                    "player-registry".to_string(),
                    Capability::RawPacket,
                    vec!["[method]player.send-packet".to_string()]
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
            err.contains("infrarust:plugin/player-registry needs `raw-packet`"),
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
        assert_eq!(gate("types", "anything"), None);
        assert_eq!(
            gate("player-registry", "[method]player.profile"),
            Some(Capability::PlayerRead)
        );
        assert_eq!(
            gate("player-registry", "[method]player.disconnect"),
            Some(Capability::PlayerWrite)
        );
        assert_eq!(gate("event-bus", "unsubscribe"), Some(Capability::EventBus));
    }
}
