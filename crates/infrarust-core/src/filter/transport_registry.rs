//! Concrete implementation of [`TransportFilterRegistry`].

use std::sync::{Arc, RwLock};

use infrarust_api::filter::{FilterMetadata, TransportFilter, TransportFilterRegistry};

use super::ordering::resolve_filter_order;
use super::transport_chain::TransportFilterChain;

/// Stores registered [`TransportFilter`] instances and maintains
/// a resolved execution order.
pub struct TransportFilterRegistryImpl {
    filters: RwLock<Vec<Arc<dyn TransportFilter>>>,
    ordered_ids: RwLock<Vec<String>>,
}

impl TransportFilterRegistryImpl {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            filters: RwLock::new(Vec::new()),
            ordered_ids: RwLock::new(Vec::new()),
        }
    }

    /// Builds a [`TransportFilterChain`] with the current filters in resolved order.
    pub fn build_chain(&self) -> TransportFilterChain {
        let filters = self.filters.read().unwrap_or_else(|e| e.into_inner());
        let ordered = self.ordered_ids.read().unwrap_or_else(|e| e.into_inner());

        let ordered_filters: Vec<Arc<dyn TransportFilter>> = ordered
            .iter()
            .filter_map(|id| filters.iter().find(|f| f.metadata().id == id).cloned())
            .collect();

        TransportFilterChain::new(ordered_filters)
    }

    /// Recalculates the ordered_ids from current filters.
    ///
    /// On cycle detection failure, logs an error and preserves the previous
    /// order. This allows the proxy to continue operating with a stale order
    /// rather than crashing.
    fn recalculate_order(&self) {
        let filters = self.filters.read().unwrap_or_else(|e| e.into_inner());
        let metadata: Vec<FilterMetadata> = filters.iter().map(|f| f.metadata()).collect();

        match resolve_filter_order(&metadata) {
            Ok(order) => {
                let mut ordered = self.ordered_ids.write().unwrap_or_else(|e| e.into_inner());
                *ordered = order;
            }
            Err(e) => {
                tracing::error!("Failed to resolve transport filter order: {e}");
            }
        }
    }
}

impl Default for TransportFilterRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl infrarust_api::filter::registry::private::Sealed for TransportFilterRegistryImpl {}

impl TransportFilterRegistry for TransportFilterRegistryImpl {
    fn register(&self, filter: Box<dyn TransportFilter>) {
        let id = filter.metadata().id;
        tracing::debug!(filter_id = id, "Registering transport filter");

        {
            let mut filters = self.filters.write().unwrap_or_else(|e| e.into_inner());
            filters.retain(|f| f.metadata().id != id);
            filters.push(Arc::from(filter));
        }

        self.recalculate_order();
    }

    fn unregister(&self, filter_id: &str) {
        tracing::debug!(filter_id, "Unregistering transport filter");

        {
            let mut filters = self.filters.write().unwrap_or_else(|e| e.into_inner());
            filters.retain(|f| f.metadata().id != filter_id);
        }

        self.recalculate_order();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use infrarust_api::event::BoxFuture;
    use infrarust_api::filter::*;

    use super::*;

    struct MockTransportFilter {
        id: &'static str,
        priority: FilterPriority,
    }

    impl TransportFilter for MockTransportFilter {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata {
                id: self.id,
                priority: self.priority,
                after: vec![],
                before: vec![],
            }
        }

        fn on_accept<'a>(&'a self, _ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
            Box::pin(async { FilterVerdict::Continue })
        }

        fn on_client_data<'a>(
            &'a self,
            _ctx: &'a mut TransportContext,
            _data: &'a mut bytes::BytesMut,
        ) -> BoxFuture<'a, FilterVerdict> {
            Box::pin(async { FilterVerdict::Continue })
        }

        fn on_server_data<'a>(
            &'a self,
            _ctx: &'a mut TransportContext,
            _data: &'a mut bytes::BytesMut,
        ) -> BoxFuture<'a, FilterVerdict> {
            Box::pin(async { FilterVerdict::Continue })
        }
    }

    #[test]
    fn test_register_and_build_chain() {
        let registry = TransportFilterRegistryImpl::new();
        registry.register(Box::new(MockTransportFilter {
            id: "filter_a",
            priority: FilterPriority::Normal,
        }));
        registry.register(Box::new(MockTransportFilter {
            id: "filter_b",
            priority: FilterPriority::First,
        }));

        let chain = registry.build_chain();
        assert!(!chain.is_empty());
    }

    #[test]
    fn test_unregister() {
        let registry = TransportFilterRegistryImpl::new();
        registry.register(Box::new(MockTransportFilter {
            id: "filter_a",
            priority: FilterPriority::Normal,
        }));

        registry.unregister("filter_a");
        let chain = registry.build_chain();
        assert!(chain.is_empty());
    }
}
