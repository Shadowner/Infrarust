use std::fmt::Display;
use std::sync::{Arc, RwLock};

use infrarust_api::services::providers::{ProviderKind, ProviderRejected};

use crate::util::sync::{read, write};

pub struct PluginProviderSlot<P: ?Sized> {
    kind: ProviderKind,
    selected: Option<String>,
    label: String,
    registered: RwLock<Option<Arc<P>>>,
}

impl<P: ?Sized> PluginProviderSlot<P> {
    pub fn new(kind: ProviderKind, selected: Option<&str>, selection: &impl Display) -> Self {
        Self {
            kind,
            selected: selected.map(str::to_owned),
            label: selection.to_string(),
            registered: RwLock::new(None),
        }
    }

    pub fn register(&self, plugin_id: &str, provider: Arc<P>) -> Result<(), ProviderRejected> {
        if self.selected.as_deref() == Some(plugin_id) {
            *write(&self.registered) = Some(provider);
            tracing::info!(plugin = %plugin_id, section = self.kind.section(), "provider registered, the service now goes through it");
            return Ok(());
        }
        tracing::warn!(
            plugin = %plugin_id,
            selected = %self.label,
            section = self.kind.section(),
            "ignoring a provider: the configuration selects another one"
        );
        Err(ProviderRejected::NotSelected {
            kind: self.kind,
            selected: self.label.clone(),
        })
    }

    pub fn unregister(&self, plugin_id: &str) -> bool {
        if self.selected.as_deref() != Some(plugin_id) {
            return false;
        }
        let removed = write(&self.registered).take().is_some();
        if removed {
            tracing::error!(
                plugin = %plugin_id,
                section = self.kind.section(),
                "the provider plugin went away, the service is degraded until it registers again"
            );
        }
        removed
    }

    pub fn get(&self) -> Option<Arc<P>> {
        read(&self.registered).clone()
    }

    pub fn report_missing(&self, consequence: &str) {
        if let Some(id) = &self.selected
            && self.get().is_none()
        {
            tracing::error!(
                plugin = %id,
                section = self.kind.section(),
                "the configured provider plugin registered no provider; {consequence} until it does"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn registers_only_the_selected_plugin() {
        let slot: PluginProviderSlot<str> =
            PluginProviderSlot::new(ProviderKind::Ban, Some("guard"), &"guard");
        assert_eq!(
            slot.register("other", Arc::from("p")),
            Err(ProviderRejected::NotSelected {
                kind: ProviderKind::Ban,
                selected: "guard".into()
            })
        );
        assert!(slot.get().is_none());
        slot.register("guard", Arc::from("p")).unwrap();
        assert_eq!(slot.get().as_deref(), Some("p"));
        assert!(!slot.unregister("other"));
        assert!(slot.get().is_some());
        assert!(slot.unregister("guard"));
        assert!(slot.get().is_none());
        assert!(!slot.unregister("guard"));
    }

    #[test]
    fn builtin_selection_rejects_every_plugin() {
        let slot: PluginProviderSlot<str> =
            PluginProviderSlot::new(ProviderKind::Permission, None, &"builtin");
        assert_eq!(
            slot.register("guard", Arc::from("p")),
            Err(ProviderRejected::NotSelected {
                kind: ProviderKind::Permission,
                selected: "builtin".into()
            })
        );
        assert!(!slot.unregister("guard"));
    }
}
