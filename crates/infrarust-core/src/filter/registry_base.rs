use std::collections::HashMap;
use std::sync::RwLock;

use infrarust_api::filter::{FilterMetadata, FilterRegistryError, PROXY_FILTER_OWNER};

use super::ordering::{FilterOrderError, resolve_filter_order};
use crate::util::sync::{read, write};

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
    registration: u64,
}

pub struct FilterRegistryBase<F: HasFilterMetadata> {
    ordered: RwLock<Vec<Registered<F>>>,
    label: &'static str,
}

impl<F: HasFilterMetadata> FilterRegistryBase<F> {
    pub fn new(label: &'static str) -> Self {
        Self {
            ordered: RwLock::new(Vec::new()),
            label,
        }
    }

    pub fn register(&self, owner: FilterOwner, item: F) -> Result<(), FilterRegistryError> {
        let metadata = item.metadata();
        let mut ordered = write(&self.ordered);
        if let Some(existing) = ordered
            .iter()
            .find(|entry| entry.metadata.id == metadata.id)
            && existing.owner != owner
        {
            return Err(FilterRegistryError::OwnedBy {
                id: metadata.id,
                owner: existing.owner.name().to_owned(),
            });
        }

        let registration = ordered
            .iter()
            .map(|entry| entry.registration + 1)
            .max()
            .unwrap_or(0);
        let positions = positions(
            ordered
                .iter()
                .filter(|entry| entry.metadata.id != metadata.id)
                .map(|entry| (entry.registration, &entry.metadata))
                .chain([(registration, &metadata)]),
        )
        .map_err(|FilterOrderError::CyclicDependency(involved)| {
            FilterRegistryError::OrderingCycle {
                id: metadata.id.clone(),
                involved,
            }
        })?;

        tracing::debug!(
            filter_id = %metadata.id,
            owner = owner.name(),
            kind = self.label,
            "Registering filter"
        );
        ordered.retain(|entry| entry.metadata.id != metadata.id);
        ordered.push(Registered {
            metadata,
            owner,
            item,
            registration,
        });
        sort(&mut ordered, &positions);
        Ok(())
    }

    pub fn unregister(
        &self,
        owner: &FilterOwner,
        filter_id: &str,
    ) -> Result<(), FilterRegistryError> {
        let mut ordered = write(&self.ordered);
        let at = ordered
            .iter()
            .position(|entry| entry.metadata.id == filter_id)
            .ok_or_else(|| FilterRegistryError::NotFound(filter_id.to_owned()))?;
        if ordered[at].owner != *owner {
            return Err(FilterRegistryError::OwnedBy {
                id: filter_id.to_owned(),
                owner: ordered[at].owner.name().to_owned(),
            });
        }
        tracing::debug!(
            filter_id,
            owner = owner.name(),
            kind = self.label,
            "Unregistering filter"
        );
        ordered.remove(at);
        self.reorder(&mut ordered);
        Ok(())
    }

    pub fn unregister_owner(&self, owner: &FilterOwner) -> usize {
        let mut ordered = write(&self.ordered);
        let before = ordered.len();
        ordered.retain(|entry| entry.owner != *owner);
        let removed = before - ordered.len();
        if removed > 0 {
            tracing::debug!(
                owner = owner.name(),
                removed,
                kind = self.label,
                "Unregistering the filters of an owner"
            );
            self.reorder(&mut ordered);
        }
        removed
    }

    pub fn owner_of(&self, filter_id: &str) -> Option<FilterOwner> {
        read(&self.ordered)
            .iter()
            .find(|entry| entry.metadata.id == filter_id)
            .map(|entry| entry.owner.clone())
    }

    pub fn owned_by(&self, owner: &FilterOwner) -> Vec<String> {
        let ordered = read(&self.ordered);
        let mut owned: Vec<&Registered<F>> = ordered
            .iter()
            .filter(|entry| entry.owner == *owner)
            .collect();
        owned.sort_unstable_by_key(|entry| entry.registration);
        owned
            .into_iter()
            .map(|entry| entry.metadata.id.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        read(&self.ordered).is_empty()
    }

    pub fn with_ordered<R>(&self, f: impl FnOnce(&[Registered<F>]) -> R) -> R {
        f(&read(&self.ordered))
    }

    fn reorder(&self, remaining: &mut [Registered<F>]) {
        match positions(
            remaining
                .iter()
                .map(|entry| (entry.registration, &entry.metadata)),
        ) {
            Ok(positions) => sort(remaining, &positions),
            Err(e) => tracing::error!("Failed to resolve {} filter order: {e}", self.label),
        }
    }
}

fn positions<'a>(
    filters: impl Iterator<Item = (u64, &'a FilterMetadata)>,
) -> Result<HashMap<String, usize>, FilterOrderError> {
    let mut filters: Vec<(u64, &FilterMetadata)> = filters.collect();
    filters.sort_unstable_by_key(|(registration, _)| *registration);
    let registered: Vec<FilterMetadata> = filters
        .into_iter()
        .map(|(_, metadata)| metadata.clone())
        .collect();
    Ok(resolve_filter_order(&registered)?
        .into_iter()
        .enumerate()
        .map(|(position, id)| (id, position))
        .collect())
}

fn sort<F>(entries: &mut [Registered<F>], positions: &HashMap<String, usize>) {
    entries.sort_by_key(|entry| positions.get(&entry.metadata.id).copied());
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use infrarust_api::filter::FilterMetadata;

    use super::*;

    struct Tagged {
        id: &'static str,
        tag: &'static str,
        after: Vec<String>,
    }

    impl HasFilterMetadata for Tagged {
        fn metadata(&self) -> FilterMetadata {
            let mut metadata = FilterMetadata::new(self.id);
            metadata.after.clone_from(&self.after);
            metadata
        }
    }

    fn filter(id: &'static str, tag: &'static str) -> Tagged {
        Tagged {
            id,
            tag,
            after: Vec::new(),
        }
    }

    fn after(id: &'static str, dependency: &str) -> Tagged {
        Tagged {
            id,
            tag: id,
            after: vec![dependency.to_owned()],
        }
    }

    fn ids(registry: &FilterRegistryBase<Tagged>) -> Vec<String> {
        tags(registry).into_iter().map(|(id, _)| id).collect()
    }

    fn tags(registry: &FilterRegistryBase<Tagged>) -> Vec<(String, &'static str)> {
        registry.with_ordered(|ordered| {
            ordered
                .iter()
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
    fn a_filter_that_closes_an_ordering_cycle_is_refused_and_the_rest_keeps_running() {
        let registry = FilterRegistryBase::new("test");
        let owner = FilterOwner::plugin("a");
        registry
            .register(owner.clone(), after("first", "second"))
            .unwrap();

        assert_eq!(
            registry.register(owner.clone(), after("second", "first")),
            Err(FilterRegistryError::OrderingCycle {
                id: "second".to_owned(),
                involved: "first, second".to_owned(),
            })
        );
        assert_eq!(ids(&registry), ["first"]);
        assert_eq!(registry.owner_of("second"), None);

        registry
            .register(owner, filter("second", "second"))
            .unwrap();
        assert_eq!(ids(&registry), ["second", "first"]);
    }

    #[test]
    fn a_replacement_that_closes_a_cycle_leaves_the_registered_filter_in_place() {
        let registry = FilterRegistryBase::new("test");
        let owner = FilterOwner::plugin("a");
        registry
            .register(owner.clone(), filter("first", "kept"))
            .unwrap();
        registry
            .register(owner.clone(), after("second", "first"))
            .unwrap();

        assert!(matches!(
            registry.register(owner, after("first", "second")),
            Err(FilterRegistryError::OrderingCycle { .. })
        ));
        assert_eq!(
            tags(&registry),
            [
                ("first".to_owned(), "kept"),
                ("second".to_owned(), "second")
            ]
        );
    }

    #[test]
    fn removing_a_dependency_falls_back_to_registration_order() {
        let registry = FilterRegistryBase::new("test");
        let owner = FilterOwner::plugin("a");
        registry
            .register(owner.clone(), after("late", "anchor"))
            .unwrap();
        registry
            .register(owner.clone(), filter("plain", "plain"))
            .unwrap();
        registry
            .register(owner.clone(), filter("anchor", "anchor"))
            .unwrap();
        assert_eq!(ids(&registry), ["plain", "anchor", "late"]);

        registry.unregister(&owner, "anchor").unwrap();
        assert_eq!(ids(&registry), ["late", "plain"]);
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
