use std::sync::RwLock;

use infrarust_api::filter::FilterMetadata;

use super::ordering::resolve_filter_order;

/// Trait for filter types that expose ordering metadata.
pub trait HasFilterMetadata {
    fn metadata(&self) -> FilterMetadata;
}

/// Generic registry that stores filters of type `F`, maintains a resolved
/// execution order, and provides register/unregister operations.
pub struct FilterRegistryBase<F: HasFilterMetadata> {
    items: RwLock<Vec<(FilterMetadata, F)>>,
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

    /// Registers a filter, replacing any existing filter with the same ID.
    pub fn register(&self, item: F) {
        let metadata = item.metadata();
        tracing::debug!(filter_id = %metadata.id, kind = self.label, "Registering filter");

        {
            let mut items = self.items.write().expect("lock poisoned");
            items.retain(|(m, _)| m.id != metadata.id);
            items.push((metadata, item));
        }

        self.recalculate_order();
    }

    pub fn unregister(&self, filter_id: &str) {
        tracing::debug!(filter_id, kind = self.label, "Unregistering filter");

        {
            let mut items = self.items.write().expect("lock poisoned");
            items.retain(|(m, _)| m.id != filter_id);
        }

        self.recalculate_order();
    }

    pub fn is_empty(&self) -> bool {
        self.items.read().expect("lock poisoned").is_empty()
    }

    /// Provides read access to the items (with their cached metadata) and
    /// ordered IDs for building output structures (chains, instance lists, etc.).
    pub fn with_ordered<R>(&self, f: impl FnOnce(&[(FilterMetadata, F)], &[String]) -> R) -> R {
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
        let metadata: Vec<FilterMetadata> = items.iter().map(|(m, _)| m.clone()).collect();

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
