use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_plugin_common::capability::gates::required;
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
        let ComponentItem::ComponentInstance(instance) = item.ty else {
            continue;
        };
        for (function, item) in instance.exports(engine) {
            if !matches!(item.ty, ComponentItem::ComponentFunc(_)) {
                continue;
            }
            for &capability in required(interface, function) {
                if granted.has(capability) {
                    continue;
                }
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
    fn packet_subscriptions_need_the_event_bus_as_well_as_raw_packets() {
        let engine = engine();
        let component = Component::new(
            &engine,
            r#"
            (component
                (import "infrarust:plugin/event-bus@0.3.0" (instance
                    (export "subscribe-packets" (func))
                ))
            )
            "#,
        )
        .unwrap();

        let only_raw = CapabilitySet::default().with(Capability::RawPacket);
        assert_eq!(
            grants(&missing_grants(&engine, &component, &only_raw)),
            [(
                "event-bus".to_string(),
                Capability::EventBus,
                vec!["subscribe-packets".to_string()]
            )]
        );
        assert!(check_imports(&engine, &component, "p", &only_raw, true).is_err());

        let only_bus = CapabilitySet::default().with(Capability::EventBus);
        assert_eq!(
            grants(&missing_grants(&engine, &component, &only_bus)),
            [(
                "event-bus".to_string(),
                Capability::RawPacket,
                vec!["subscribe-packets".to_string()]
            )]
        );

        let both = only_bus.with(Capability::RawPacket);
        assert!(missing_grants(&engine, &component, &both).is_empty());
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
        assert!(required("limbo", "[method]limbo-session.send-message").is_empty());
        assert_eq!(
            required("limbo", "register-limbo-handler"),
            [Capability::Limbo]
        );
        assert!(required("log", "info").is_empty());
        assert!(required("text", "parse-json").is_empty());
        assert!(required("types", "anything").is_empty());
        assert!(required("events", "anything").is_empty());
        assert_eq!(required("players", "get"), [Capability::PlayerRead]);
        assert_eq!(
            required("players", "has-permission"),
            [Capability::PlayerRead]
        );
        assert_eq!(required("players", "disconnect"), [Capability::PlayerWrite]);
        assert_eq!(required("players", "send-packet"), [Capability::RawPacket]);
        assert_eq!(required("event-bus", "unsubscribe"), [Capability::EventBus]);
        assert_eq!(required("event-bus", "fire-named"), [Capability::EventBus]);
        assert_eq!(
            required("event-bus", "subscribe-packets"),
            [Capability::EventBus, Capability::RawPacket]
        );
        assert_eq!(required("players", "connect"), [Capability::PlayerWrite]);
        assert_eq!(
            required("players", "request-cookie"),
            [Capability::PlayerWrite]
        );
        assert_eq!(
            required("messaging", "send-to-server"),
            [Capability::PluginMessaging]
        );
        assert_eq!(
            required("load-balancer", "backends"),
            [Capability::ConfigRead]
        );
        assert_eq!(
            required("load-balancer", "set-drained"),
            [Capability::ServerManage]
        );
        assert_eq!(
            required("config-service", "write-proxy-config-document"),
            [Capability::ConfigWrite]
        );
        assert_eq!(
            required("config-service", "list-server-sources"),
            [Capability::ConfigRead]
        );
        assert!(required("proxy-info", "granted-capabilities").is_empty());
        assert!(required("plugin-registry", "list").is_empty());
        assert_eq!(
            required("providers", "register-ban-provider"),
            [Capability::BanProvider]
        );
        assert_eq!(
            required("providers", "register-permission-provider"),
            [Capability::PermissionProvider]
        );
        assert_eq!(
            required("permissions", "set-snapshot"),
            [Capability::PermissionProvider]
        );
        assert_eq!(
            required("permissions", "release"),
            [Capability::PermissionProvider]
        );
    }
}
