use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use infrarust_api::permissions::{
    ADMIN_PERMISSION, COMMAND_PERMISSION_PREFIX, PermissionChecker, PermissionDefault,
    PermissionNode, PermissionNodeError, PermissionNodeInfo, Tristate, WILDCARD, normalize_node,
};

const RESERVED_PREFIX: &str = "infrarust.";

pub fn command_node(command: &str) -> String {
    format!("{COMMAND_PERMISSION_PREFIX}{}", normalize_node(command))
}

struct Entry {
    node: PermissionNode,
    owner: Option<String>,
}

#[derive(Default)]
pub(crate) struct NodeRegistry {
    entries: RwLock<HashMap<String, Entry>>,
}

impl NodeRegistry {
    pub(crate) fn register(
        &self,
        owner: Option<&str>,
        node: PermissionNode,
    ) -> Result<(), PermissionNodeError> {
        let name = normalize_node(&node.name).into_owned();
        if !is_valid(&name) {
            return Err(PermissionNodeError::InvalidName(node.name));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(owner) = owner {
            if name.starts_with(RESERVED_PREFIX) {
                return Err(PermissionNodeError::Reserved(name));
            }
            match entries.get(&name).map(|entry| entry.owner.as_deref()) {
                Some(None) => return Err(PermissionNodeError::Reserved(name)),
                Some(Some(existing)) if existing != owner => {
                    return Err(PermissionNodeError::OwnedBy {
                        name,
                        plugin: existing.to_string(),
                    });
                }
                _ => {}
            }
        }
        let mut node = node;
        node.name.clone_from(&name);
        entries.insert(
            name,
            Entry {
                node,
                owner: owner.map(str::to_string),
            },
        );
        Ok(())
    }

    pub(crate) fn unregister_owned(&self, owner: &str) -> usize {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        let before = entries.len();
        entries.retain(|_, entry| entry.owner.as_deref() != Some(owner));
        before - entries.len()
    }

    pub(crate) fn default_of(&self, node: &str) -> Option<PermissionDefault> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(normalize_node(node).as_ref())
            .map(|entry| entry.node.default)
    }

    pub(crate) fn list(&self) -> Vec<PermissionNodeInfo> {
        let mut infos: Vec<PermissionNodeInfo> = self
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|entry| PermissionNodeInfo::new(entry.node.clone(), entry.owner.clone()))
            .collect();
        infos.sort_by(|a, b| a.node.name.cmp(&b.node.name));
        infos
    }

    pub(crate) fn resolve(&self, checker: &dyn PermissionChecker, node: &str) -> Tristate {
        let node = normalize_node(node);
        match checker.value(&node) {
            Tristate::Undefined => match self.default_of(&node) {
                Some(PermissionDefault::True) => Tristate::True,
                Some(PermissionDefault::False) => Tristate::False,
                Some(PermissionDefault::Admin) => {
                    Tristate::from_bool(checker.value(ADMIN_PERMISSION).is_true())
                }
                _ => Tristate::Undefined,
            },
            defined => defined,
        }
    }
}

fn is_valid(name: &str) -> bool {
    !name.is_empty()
        && name != WILDCARD
        && !name.ends_with(".*")
        && name
            .split('.')
            .all(|segment| !segment.is_empty() && !segment.chars().any(char::is_whitespace))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use infrarust_api::permissions::{
        AllPermissionsChecker, DefaultPermissionChecker, PermissionMap,
    };

    use super::*;

    fn registry() -> NodeRegistry {
        let nodes = NodeRegistry::default();
        nodes
            .register(
                None,
                PermissionNode::new(ADMIN_PERMISSION, PermissionDefault::False),
            )
            .unwrap();
        for (name, default) in [
            ("demo.open", PermissionDefault::True),
            ("demo.closed", PermissionDefault::False),
            ("demo.staff", PermissionDefault::Admin),
        ] {
            nodes
                .register(Some("demo"), PermissionNode::new(name, default))
                .unwrap();
        }
        nodes
    }

    #[test]
    fn a_defined_value_wins_over_the_default() {
        let nodes = registry();
        let checker = PermissionMap::new()
            .with("demo.open", false)
            .with("demo.closed", true);
        assert_eq!(nodes.resolve(&checker, "demo.open"), Tristate::False);
        assert_eq!(nodes.resolve(&checker, "demo.closed"), Tristate::True);
    }

    #[test]
    fn an_undefined_value_falls_back_to_the_node_default() {
        let nodes = registry();
        let player = DefaultPermissionChecker;
        assert_eq!(nodes.resolve(&player, "demo.open"), Tristate::True);
        assert_eq!(nodes.resolve(&player, "demo.closed"), Tristate::False);
        assert_eq!(nodes.resolve(&player, "demo.staff"), Tristate::False);
        assert_eq!(
            nodes.resolve(&player, "never.registered"),
            Tristate::Undefined
        );
        assert_eq!(nodes.resolve(&player, ADMIN_PERMISSION), Tristate::False);
    }

    #[test]
    fn an_admin_default_follows_the_subjects_admin_node() {
        let nodes = registry();
        let admin = PermissionMap::new().with(ADMIN_PERMISSION, true);
        assert_eq!(nodes.resolve(&admin, "demo.staff"), Tristate::True);
        assert_eq!(nodes.resolve(&admin, "demo.closed"), Tristate::False);
        assert_eq!(
            nodes.resolve(&admin, "never.registered"),
            Tristate::Undefined
        );
        let everything = AllPermissionsChecker;
        assert_eq!(
            nodes.resolve(&everything, "never.registered"),
            Tristate::True
        );
    }

    #[test]
    fn nodes_are_looked_up_case_insensitively() {
        let nodes = registry();
        assert_eq!(
            nodes.resolve(&DefaultPermissionChecker, "Demo.OPEN"),
            Tristate::True
        );
        let checker = PermissionMap::new().with("demo.closed", true);
        assert_eq!(nodes.resolve(&checker, " DEMO.Closed "), Tristate::True);
    }

    #[test]
    fn plugins_cannot_take_proxy_or_foreign_nodes() {
        let nodes = registry();
        assert_eq!(
            nodes.register(
                Some("other"),
                PermissionNode::new("infrarust.fly", PermissionDefault::True)
            ),
            Err(PermissionNodeError::Reserved("infrarust.fly".into()))
        );
        assert_eq!(
            nodes.register(
                Some("other"),
                PermissionNode::new("Demo.Open", PermissionDefault::False)
            ),
            Err(PermissionNodeError::OwnedBy {
                name: "demo.open".into(),
                plugin: "demo".into()
            })
        );
        nodes
            .register(
                Some("demo"),
                PermissionNode::new("demo.open", PermissionDefault::False),
            )
            .unwrap();
        assert_eq!(
            nodes.default_of("demo.open"),
            Some(PermissionDefault::False)
        );
        for invalid in ["", "*", "demo.*", "a..b", "a b", ".a"] {
            assert!(
                matches!(
                    nodes.register(
                        Some("demo"),
                        PermissionNode::new(invalid, PermissionDefault::True)
                    ),
                    Err(PermissionNodeError::InvalidName(_))
                ),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn a_disabled_plugin_takes_its_nodes_along() {
        let nodes = registry();
        assert_eq!(nodes.unregister_owned("demo"), 3);
        assert_eq!(nodes.default_of("demo.open"), None);
        assert_eq!(
            nodes
                .list()
                .iter()
                .map(|i| i.node.name.clone())
                .collect::<Vec<_>>(),
            [ADMIN_PERMISSION]
        );
    }
}
