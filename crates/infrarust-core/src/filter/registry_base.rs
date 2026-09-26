use std::sync::RwLock;

use infrarust_api::filter::{FilterMetadata, FilterRegistryError, PROXY_FILTER_OWNER};

use super::ordering::resolve_filter_order;

/// Trait for filter types that expose ordering metadata.
pub trait HasFilterMetadata {
    fn metadata(&self) -> FilterMetadata;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterOwner {
    Proxy,
    Plugin(String),
}

impl FilterOwner {
    pub fn plugin(plugin_id: impl Into<String>) -> Self {
        Self::Plugin(plugin_id.into())
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Proxy => PROXY_FILTER_OWNER,
            Self::Plugin(plugin_id) => plugin_id,
        }
    }
}

pub struct Registered<F> {
    pub metadata: FilterMetadata,
    pub owner: FilterOwner,
    pub item: F,
}

/// Generic registry that stores filters of type `F`, maintains a resolved
/// execution order, and provides register/unregister operations.
pub struct FilterRegistryBase<F: HasFilterMetadata> {
    items: RwLock<Vec<Registered<F>>>,
    ordered_ids: RwLock<Vec<String>>,
    label: &'static str,
}

impl<F: HasFilterMetadata> FilterRegistryBase<F> {
    pub fn new(label: &'static str) -> Self {
        Self {
            items: RwLock::new(Vec::new()),
            ordered_ids: RwLock::new(Vec::new()),
            label,
        }
    }

    pub fn register(&self, owner: FilterOwner, item: F) -> Result<(), FilterRegistryError> {
        let metadata = item.metadata();
        {
            let mut items = self.items.write().expect("lock poisoned");
            if let Some(existing) = items.iter().find(|entry| entry.metadata.id == metadata.id)
                && existing.owner != owner
            {
                return Err(FilterRegistryError::OwnedBy {
                    id: metadata.id,
                    owner: existing.owner.name().to_owned(),
                });
            }
            tracing::debug!(
                filter_id = %metadata.id,
                owner = owner.name(),
                kind = self.label,
                "Registering filter"
            );
            items.retain(|entry| entry.metadata.id != metadata.id);
            items.push(Registered {
                metadata,
                owner,
                item,
            });
        }

        self.recalculate_order();
        Ok(())
    }

    pub fn unregister(
        &self,
        owner: &FilterOwner,
        filter_id: &str,
    ) -> Result<(), FilterRegistryError> {
        {
            let mut items = self.items.write().expect("lock poisoned");
            let at = items
                .iter()
                .position(|entry| entry.metadata.id == filter_id)
                .ok_or_else(|| FilterRegistryError::NotFound(filter_id.to_owned()))?;
            if items[at].owner != *owner {
                return Err(FilterRegistryError::OwnedBy {
                    id: filter_id.to_owned(),
                    owner: items[at].owner.name().to_owned(),
                });
            }
            tracing::debug!(
                filter_id,
                owner = owner.name(),
                kind = self.label,
                "Unregistering filter"
            );
            items.remove(at);
        }

        self.recalculate_order();
        Ok(())
    }

    pub fn unregister_owner(&self, owner: &FilterOwner) -> usize {
        let removed = {
            let mut items = self.items.write().expect("lock poisoned");
            let before = items.len();
            items.retain(|entry| entry.owner != *owner);
            before - items.len()
        };
        if removed > 0 {
            tracing::debug!(
                owner = owner.name(),
                removed,
                kind = self.label,
                "Unregistering the filters of an owner"
            );
            self.recalculate_order();
        }
        removed
    }

    pub fn owner_of(&self, filter_id: &str) -> Option<FilterOwner> {
        self.items
            .read()
            .expect("lock poisoned")
            .iter()
            .find(|entry| entry.metadata.id == filter_id)
            .map(|entry| entry.owner.clone())
    }

    pub fn owned_by(&self, owner: &FilterOwner) -> Vec<String> {
        self.items
            .read()
            .expect("lock poisoned")
            .iter()
            .filter(|entry| entry.owner == *owner)
            .map(|entry| entry.metadata.id.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.items.read().expect("lock poisoned").is_empty()
    }

    /// Provides read access to the items (with their cached metadata) and
    /// ordered IDs for building output structures (chains, instance lists, etc.).
    pub fn with_ordered<R>(&self, f: impl FnOnce(&[Registered<F>], &[String]) -> R) -> R {
        let items = self.items.read().expect("lock poisoned");
        let ordered = self.ordered_ids.read().expect("lock poisoned");
        f(&items, &ordered)
    }

    /// Recalculates the ordered IDs from current items.
    ///
    /// On cycle detection failure, logs an error and preserves the previous
    /// order so the proxy continues operating with a stale order.
    fn recalculate_order(&self) {
        let items = self.items.read().expect("lock poisoned");
        let metadata: Vec<FilterMetadata> =
            items.iter().map(|entry| entry.metadata.clone()).collect();

        match resolve_filter_order(&metadata) {
            Ok(order) => {
                let mut ordered = self.ordered_ids.write().expect("lock poisoned");
                *ordered = order;
            }
            Err(e) => {
                tracing::error!("Failed to resolve {} filter order: {e}", self.label);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use infrarust_api::filter::FilterMetadata;

    use super::*;

    struct Tagged {
        id: &'static str,
        tag: &'static str,
    }

    impl HasFilterMetadata for Tagged {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata::new(self.id)
        }
    }

    fn filter(id: &'static str, tag: &'static str) -> Tagged {
        Tagged { id, tag }
    }

    fn tags(registry: &FilterRegistryBase<Tagged>) -> Vec<(String, &'static str)> {
        registry.with_ordered(|items, ordered| {
            ordered
                .iter()
                .filter_map(|id| items.iter().find(|entry| entry.metadata.id == *id))
                .map(|entry| (entry.metadata.id.clone(), entry.item.tag))
                .collect()
        })
    }

    fn owned_by(id: &str, owner: &str) -> FilterRegistryError {
        FilterRegistryError::OwnedBy {
            id: id.to_owned(),
            owner: owner.to_owned(),
        }
    }

    #[test]
    fn a_plugin_cannot_replace_or_remove_another_plugins_filter() {
        let registry = FilterRegistryBase::new("test");
        let (a, b) = (FilterOwner::plugin("a"), FilterOwner::plugin("b"));
        registry.register(a.clone(), filter("shared", "a")).unwrap();

        assert_eq!(
            registry.register(b.clone(), filter("shared", "b")),
            Err(owned_by("shared", "a"))
        );
        assert_eq!(
            registry.unregister(&b, "shared"),
            Err(owned_by("shared", "a"))
        );
        assert_eq!(tags(&registry), [("shared".to_owned(), "a")]);
        assert_eq!(registry.owner_of("shared"), Some(a));
    }

    #[test]
    fn a_plugin_replaces_and_removes_its_own_filter() {
        let registry = FilterRegistryBase::new("test");
        let a = FilterOwner::plugin("a");
        registry
            .register(a.clone(), filter("mine", "first"))
            .unwrap();
        registry
            .register(a.clone(), filter("mine", "second"))
            .unwrap();
        assert_eq!(tags(&registry), [("mine".to_owned(), "second")]);

        registry.unregister(&a, "mine").unwrap();
        assert!(registry.is_empty());
        assert!(tags(&registry).is_empty());
        assert_eq!(
            registry.unregister(&a, "mine"),
            Err(FilterRegistryError::NotFound("mine".to_owned()))
        );
    }

    #[test]
    fn a_proxy_filter_is_reserved_against_every_plugin() {
        let registry = FilterRegistryBase::new("test");
        let plugin = FilterOwner::plugin(PROXY_FILTER_OWNER);
        registry
            .register(FilterOwner::Proxy, filter("core", "proxy"))
            .unwrap();

        assert_eq!(
            registry.register(plugin.clone(), filter("core", "plugin")),
            Err(owned_by("core", PROXY_FILTER_OWNER))
        );
        assert_eq!(
            registry.unregister(&plugin, "core"),
            Err(owned_by("core", PROXY_FILTER_OWNER))
        );
        assert_eq!(registry.unregister_owner(&plugin), 0);
        assert_eq!(tags(&registry), [("core".to_owned(), "proxy")]);
        assert_eq!(registry.owner_of("core"), Some(FilterOwner::Proxy));
    }

    #[test]
    fn removing_an_owner_takes_only_its_filters_out_of_the_order() {
        let registry = FilterRegistryBase::new("test");
        let (a, b) = (FilterOwner::plugin("a"), FilterOwner::plugin("b"));
        registry.register(a.clone(), filter("a1", "a")).unwrap();
        registry.register(b.clone(), filter("b1", "b")).unwrap();
        registry.register(a.clone(), filter("a2", "a")).unwrap();
        registry
            .register(FilterOwner::Proxy, filter("core", "proxy"))
            .unwrap();
        assert_eq!(registry.owned_by(&a), ["a1", "a2"]);

        assert_eq!(registry.unregister_owner(&a), 2);
        assert_eq!(
            tags(&registry),
            [("b1".to_owned(), "b"), ("core".to_owned(), "proxy")]
        );
        assert!(registry.owned_by(&a).is_empty());
        assert_eq!(registry.unregister_owner(&a), 0);
    }
}
