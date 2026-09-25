use std::collections::BTreeSet;
use std::path::Path;

use wit_parser::{Function, Interface, InterfaceId, PackageId, Resolve, Type, TypeDefKind, TypeId};

const GATED: &[&str] = &[
    "event-bus",
    "players",
    "server-manager",
    "ban-service",
    "config-service",
    "command-manager",
    "scheduler",
    "codec-registry",
    "limbo",
    "load-balancer",
    "messaging",
];

const INFALLIBLE_READS: &[(&str, &str)] = &[
    ("players", "get"),
    ("players", "get-by-name"),
    ("players", "get-by-uuid"),
    ("players", "list"),
    ("players", "count"),
    ("limbo", "[method]limbo-session.player-id"),
    ("limbo", "[method]limbo-session.profile"),
    ("limbo", "[method]limbo-session.entry-context"),
    ("limbo", "[method]limbo-session.acquire-handle"),
    ("limbo", "[method]limbo-session-handle.player-id"),
    ("limbo", "[method]limbo-session-handle.cancelled"),
];

struct Contract {
    resolve: Resolve,
    package: PackageId,
}

impl Contract {
    fn load() -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(infrarust_plugin_wit::WIT_DIR);
        let mut resolve = Resolve::new();
        let (package, _) = resolve.push_dir(&dir).expect("the contract parses");
        Self { resolve, package }
    }

    fn interface_id(&self, name: &str) -> InterfaceId {
        *self.resolve.packages[self.package]
            .interfaces
            .get(name)
            .unwrap_or_else(|| panic!("interface `{name}` is missing"))
    }

    fn interface(&self, name: &str) -> &Interface {
        &self.resolve.interfaces[self.interface_id(name)]
    }

    fn type_id(&self, interface: &str, name: &str) -> TypeId {
        *self
            .interface(interface)
            .types
            .get(name)
            .unwrap_or_else(|| panic!("type `{interface}.{name}` is missing"))
    }

    fn canonical(&self, mut id: TypeId) -> TypeId {
        while let TypeDefKind::Type(Type::Id(next)) = self.resolve.types[id].kind {
            id = next;
        }
        id
    }

    fn named(&self, ty: Type) -> Option<String> {
        let Type::Id(id) = ty else { return None };
        self.resolve.types[self.canonical(id)].name.clone()
    }

    fn kind(&self, interface: &str, name: &str) -> &TypeDefKind {
        &self.resolve.types[self.canonical(self.type_id(interface, name))].kind
    }

    fn enum_cases(&self, interface: &str, name: &str) -> Vec<String> {
        match self.kind(interface, name) {
            TypeDefKind::Enum(e) => e.cases.iter().map(|c| c.name.clone()).collect(),
            other => panic!("`{interface}.{name}` is not an enum: {other:?}"),
        }
    }

    fn variant_cases(&self, interface: &str, name: &str) -> Vec<(String, Option<String>)> {
        match self.kind(interface, name) {
            TypeDefKind::Variant(v) => v
                .cases
                .iter()
                .map(|c| (c.name.clone(), c.ty.and_then(|ty| self.named(ty))))
                .collect(),
            other => panic!("`{interface}.{name}` is not a variant: {other:?}"),
        }
    }

    fn record_fields(&self, interface: &str, name: &str) -> Vec<(String, Type)> {
        match self.kind(interface, name) {
            TypeDefKind::Record(r) => r.fields.iter().map(|f| (f.name.clone(), f.ty)).collect(),
            other => panic!("`{interface}.{name}` is not a record: {other:?}"),
        }
    }

    fn returns_host_error(&self, function: &Function) -> bool {
        let Some(Type::Id(id)) = function.result else {
            return false;
        };
        let TypeDefKind::Result(result) = &self.resolve.types[self.canonical(id)].kind else {
            return false;
        };
        let host_error = self.canonical(self.type_id("types", "host-error"));
        matches!(result.err, Some(Type::Id(err)) if self.canonical(err) == host_error)
    }
}

#[test]
fn the_package_version_is_the_world_version() {
    let contract = Contract::load();
    let name = &contract.resolve.packages[contract.package].name;
    assert_eq!(
        format!("{}:{}", name.namespace, name.name),
        infrarust_plugin_wit::PACKAGE
    );
    assert_eq!(
        name.version.as_ref().map(ToString::to_string).as_deref(),
        Some(infrarust_plugin_wit::WORLD_VERSION)
    );
}

#[test]
fn every_event_kind_has_one_event_case_with_its_record() {
    let contract = Contract::load();
    let kinds = contract.enum_cases("events", "event-kind");
    let cases = contract.variant_cases("events", "event");
    let names: Vec<&String> = cases.iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        kinds.iter().collect::<Vec<_>>(),
        "event-kind and event must list the same events in the same order"
    );
    for (name, payload) in &cases {
        if let Some(payload) = payload {
            assert_eq!(
                payload,
                &format!("{name}-event"),
                "the `{name}` event carries a `{name}-event` record"
            );
            contract.record_fields("events", payload);
        }
    }
}

#[test]
fn every_resulted_event_ends_with_its_current_result_and_has_one_outcome() {
    let contract = Contract::load();
    let outcomes = contract.variant_cases("events", "event-outcome");
    assert_eq!(
        outcomes.first(),
        Some(&("unchanged".to_owned(), None)),
        "event-outcome starts with a payload-less `unchanged`"
    );
    let mut resulted = BTreeSet::new();
    for (name, payload) in contract.variant_cases("events", "event") {
        let Some(record) = payload else { continue };
        let fields = contract.record_fields("events", &record);
        let Some(at) = fields.iter().position(|(field, _)| field == "result") else {
            continue;
        };
        assert_eq!(
            at,
            fields.len() - 1,
            "`result` is the last field of `{record}`"
        );
        assert_eq!(
            contract.named(fields[at].1),
            Some(format!("{name}-result")),
            "`{record}.result` holds a `{name}-result`"
        );
        resulted.insert(name);
    }
    let mut outcome_cases = BTreeSet::new();
    for (name, payload) in outcomes.into_iter().skip(1) {
        assert_eq!(
            payload,
            Some(format!("{name}-result")),
            "the `{name}` outcome carries a `{name}-result`"
        );
        outcome_cases.insert(name);
    }
    assert_eq!(
        outcome_cases, resulted,
        "an event has an outcome case exactly when it carries a result"
    );
}

#[test]
fn every_gated_function_returns_a_host_error() {
    let contract = Contract::load();
    let mut offenders = Vec::new();
    for interface in GATED {
        for (name, function) in &contract.interface(interface).functions {
            let allowed = INFALLIBLE_READS.contains(&(*interface, name.as_str()));
            if !allowed && !contract.returns_host_error(function) {
                offenders.push(format!("{interface}.{name}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "functions of gated interfaces must return result<_, host-error>: {offenders:?}"
    );
    for (interface, name) in INFALLIBLE_READS {
        assert!(
            contract.interface(interface).functions.contains_key(*name),
            "stale allow-list entry {interface}.{name}"
        );
    }
}

#[test]
fn the_world_imports_every_interface_and_exports_the_guest() {
    let contract = Contract::load();
    let world_id = contract.resolve.packages[contract.package].worlds["plugin"];
    let world = &contract.resolve.worlds[world_id];
    let key_names = |items: &mut dyn Iterator<Item = &wit_parser::WorldKey>| -> BTreeSet<String> {
        items
            .map(|key| contract.resolve.name_world_key(key))
            .collect()
    };
    let exports = key_names(&mut world.exports.keys());
    let expected_exports: BTreeSet<String> = ["guest", "codec-filter"]
        .iter()
        .map(|name| qualified(&contract, name))
        .collect();
    assert_eq!(exports, expected_exports);
    let imports = key_names(&mut world.imports.keys());
    for name in contract.resolve.packages[contract.package]
        .interfaces
        .keys()
    {
        if !expected_exports.contains(&qualified(&contract, name)) {
            assert!(
                imports.contains(&qualified(&contract, name)),
                "the world does not import `{name}`"
            );
        }
    }
}

fn qualified(contract: &Contract, interface: &str) -> String {
    contract
        .resolve
        .id_of(contract.interface_id(interface))
        .expect("a named interface has an id")
}
