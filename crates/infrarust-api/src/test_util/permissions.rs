use std::sync::Mutex;

use crate::permissions::{PermissionChecker, PermissionMap, Tristate, WILDCARD};

use super::lock;

#[derive(Debug, Default)]
pub struct MockPermissionChecker {
    map: Mutex<PermissionMap>,
    checked: Mutex<Vec<String>>,
}

impl MockPermissionChecker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn allow_all() -> Self {
        Self::new().grant(WILDCARD)
    }

    #[must_use]
    pub fn grant(self, node: &str) -> Self {
        self.set(node, true);
        self
    }

    #[must_use]
    pub fn deny(self, node: &str) -> Self {
        self.set(node, false);
        self
    }

    pub fn set(&self, node: &str, value: bool) {
        lock(&self.map).set(node, value);
    }

    pub fn unset(&self, node: &str) {
        lock(&self.map).unset(node);
    }

    #[must_use]
    pub fn checked(&self) -> Vec<String> {
        lock(&self.checked).clone()
    }
}

impl PermissionChecker for MockPermissionChecker {
    fn value(&self, node: &str) -> Tristate {
        lock(&self.checked).push(node.to_string());
        lock(&self.map).value(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_wildcards_and_records_checks() {
        let checker = MockPermissionChecker::new()
            .grant("auth.admin.*")
            .deny("auth.admin.purge");
        assert!(checker.has_permission("auth.admin.kick"));
        assert!(!checker.has_permission("auth.admin.purge"));
        assert_eq!(checker.value("other"), Tristate::Undefined);
        assert_eq!(
            checker.checked(),
            ["auth.admin.kick", "auth.admin.purge", "other"]
        );
        assert!(MockPermissionChecker::allow_all().has_permission("anything"));
    }
}
