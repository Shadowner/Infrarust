//! Concrete implementation of [`TransportFilterRegistry`].

use std::sync::Arc;

use infrarust_api::filter::{FilterMetadata, TransportFilter, TransportFilterRegistry};

use super::registry_base::{FilterRegistryBase, HasFilterMetadata};
use super::transport_chain::TransportFilterChain;

impl HasFilterMetadata for Arc<dyn TransportFilter> {
    fn metadata(&self) -> FilterMetadata {
        TransportFilter::metadata(self.as_ref())
    }
}

/// Stores registered [`TransportFilter`] instances and maintains
/// a resolved execution order.
pub struct TransportFilterRegistryImpl {
    base: FilterRegistryBase<Arc<dyn TransportFilter>>,
}

impl TransportFilterRegistryImpl {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: FilterRegistryBase::new("transport"),
        }
    }

    /// Builds a [`TransportFilterChain`] with the current filters in resolved order.
    pub fn build_chain(&self) -> TransportFilterChain {
        self.base.with_ordered(|filters, ordered| {
            let ordered_filters: Vec<Arc<dyn TransportFilter>> = ordered
                .iter()
                .filter_map(|id| filters.iter().find(|f| TransportFilter::metadata(f.as_ref()).id == id).cloned())
                .collect();

            TransportFilterChain::new(ordered_filters)
        })
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
        self.base.register(Arc::from(filter));
    }

    fn unregister(&self, filter_id: &str) {
        self.base.unregister(filter_id);
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
