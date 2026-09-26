//! Owner-aware store behind [`TransportFilterRegistry`](infrarust_api::filter::TransportFilterRegistry).

use std::sync::{Arc, RwLock};

use infrarust_api::filter::{FilterMetadata, FilterRegistryError, TransportFilter};

use super::registry_base::{FilterOwner, FilterRegistryBase, HasFilterMetadata};
use super::transport_chain::{ChainedFilter, TransportFilterChain};

impl HasFilterMetadata for Arc<dyn TransportFilter> {
    fn metadata(&self) -> FilterMetadata {
        TransportFilter::metadata(self.as_ref())
    }
}

/// Stores registered [`TransportFilter`] instances and maintains
/// a resolved execution order.
pub struct TransportFilterRegistryImpl {
    base: FilterRegistryBase<Arc<dyn TransportFilter>>,
    chain: RwLock<TransportFilterChain>,
}

impl TransportFilterRegistryImpl {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: FilterRegistryBase::new("transport"),
            chain: RwLock::new(TransportFilterChain::empty()),
        }
    }

    #[must_use]
    pub fn chain(&self) -> TransportFilterChain {
        self.chain.read().expect("lock poisoned").clone()
    }

    pub fn register_builtin(
        &self,
        filter: Box<dyn TransportFilter>,
    ) -> Result<(), FilterRegistryError> {
        self.changed(self.base.register(FilterOwner::Proxy, Arc::from(filter)))
    }

    pub fn unregister_builtin(&self, filter_id: &str) -> Result<(), FilterRegistryError> {
        self.changed(self.base.unregister(&FilterOwner::Proxy, filter_id))
    }

    pub fn register_owned(
        &self,
        plugin_id: &str,
        filter: Box<dyn TransportFilter>,
    ) -> Result<(), FilterRegistryError> {
        self.changed(
            self.base
                .register(FilterOwner::plugin(plugin_id), Arc::from(filter)),
        )
    }

    pub fn unregister_owned(
        &self,
        plugin_id: &str,
        filter_id: &str,
    ) -> Result<(), FilterRegistryError> {
        self.changed(
            self.base
                .unregister(&FilterOwner::plugin(plugin_id), filter_id),
        )
    }

    pub fn unregister_owner(&self, plugin_id: &str) -> usize {
        let removed = self.base.unregister_owner(&FilterOwner::plugin(plugin_id));
        if removed > 0 {
            self.rebuild();
        }
        removed
    }

    #[must_use]
    pub fn owner_of(&self, filter_id: &str) -> Option<FilterOwner> {
        self.base.owner_of(filter_id)
    }

    #[must_use]
    pub fn owned_by(&self, plugin_id: &str) -> Vec<String> {
        self.base.owned_by(&FilterOwner::plugin(plugin_id))
    }

    fn changed(&self, outcome: Result<(), FilterRegistryError>) -> Result<(), FilterRegistryError> {
        if outcome.is_ok() {
            self.rebuild();
        }
        outcome
    }

    fn rebuild(&self) {
        let mut chain = self.chain.write().expect("lock poisoned");
        *chain = self.base.with_ordered(|filters, ordered| {
            let ordered_filters: Vec<ChainedFilter> = ordered
                .iter()
                .filter_map(|id| {
                    filters
                        .iter()
                        .find(|entry| entry.metadata.id == *id)
                        .map(|entry| ChainedFilter {
                            id: entry.metadata.id.clone(),
                            owner: entry.owner.clone(),
                            filter: Arc::clone(&entry.item),
                        })
                })
                .collect();

            TransportFilterChain::new(ordered_filters)
        });
    }
}

impl Default for TransportFilterRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::time::{Duration, Instant};

    use infrarust_api::event::BoxFuture;
    use infrarust_api::filter::*;
    use infrarust_api::types::Extensions;

    use super::*;

    struct MockTransportFilter {
        id: &'static str,
        priority: FilterPriority,
        verdict: FilterVerdict,
    }

    impl TransportFilter for MockTransportFilter {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata {
                id: self.id.to_string(),
                priority: self.priority,
                after: vec![],
                before: vec![],
            }
        }

        fn on_accept<'a>(&'a self, _ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
            let verdict = self.verdict;
            Box::pin(async move { verdict })
        }
    }

    fn mock(id: &'static str, priority: FilterPriority) -> Box<dyn TransportFilter> {
        Box::new(MockTransportFilter {
            id,
            priority,
            verdict: FilterVerdict::Continue,
        })
    }

    fn rejecting(id: &'static str) -> Box<dyn TransportFilter> {
        Box::new(MockTransportFilter {
            id,
            priority: FilterPriority::Normal,
            verdict: FilterVerdict::Reject,
        })
    }

    async fn verdict(chain: &TransportFilterChain) -> FilterVerdict {
        let ctx = TransportContext {
            remote_addr: "127.0.0.1:12345".parse().unwrap(),
            local_addr: "0.0.0.0:25565".parse().unwrap(),
            real_ip: None,
            connection_time: Instant::now(),
            connection_id: 1,
            extensions: Extensions::new(),
        };
        match chain.open(ctx, Duration::from_secs(5)).await {
            Ok(_) => FilterVerdict::Continue,
            Err(_) => FilterVerdict::Reject,
        }
    }

    #[test]
    fn test_register_and_build_chain() {
        let registry = TransportFilterRegistryImpl::new();
        registry
            .register_builtin(mock("filter_a", FilterPriority::Normal))
            .unwrap();
        registry
            .register_builtin(mock("filter_b", FilterPriority::First))
            .unwrap();

        assert!(!registry.chain().is_empty());
    }

    #[test]
    fn test_unregister() {
        let registry = TransportFilterRegistryImpl::new();
        registry
            .register_builtin(mock("filter_a", FilterPriority::Normal))
            .unwrap();

        registry.unregister_builtin("filter_a").unwrap();
        assert!(registry.chain().is_empty());
    }

    #[tokio::test]
    async fn the_cached_chain_follows_ownership_changes() {
        let registry = TransportFilterRegistryImpl::new();
        registry.register_owned("owner", rejecting("gate")).unwrap();
        assert_eq!(verdict(&registry.chain()).await, FilterVerdict::Reject);

        assert_eq!(
            registry.register_owned("thief", mock("gate", FilterPriority::Normal)),
            Err(FilterRegistryError::OwnedBy {
                id: "gate".into(),
                owner: "owner".into()
            })
        );
        assert_eq!(
            registry.unregister_owned("thief", "gate"),
            Err(FilterRegistryError::OwnedBy {
                id: "gate".into(),
                owner: "owner".into()
            })
        );
        assert_eq!(verdict(&registry.chain()).await, FilterVerdict::Reject);

        assert_eq!(registry.unregister_owner("owner"), 1);
        assert!(registry.chain().is_empty());
        assert_eq!(verdict(&registry.chain()).await, FilterVerdict::Continue);
        assert_eq!(
            registry.unregister_owned("owner", "gate"),
            Err(FilterRegistryError::NotFound("gate".into()))
        );
    }
}
