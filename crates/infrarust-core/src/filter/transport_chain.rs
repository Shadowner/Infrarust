//! Transport filter chain.

use std::sync::Arc;

use infrarust_api::filter::{FilterVerdict, TransportContext, TransportFilter};

/// A chain of [`TransportFilter`]s applied to each connection.
///
/// Filters are shared (`Arc`) and the chain is cloned per-connection.
#[derive(Clone)]
pub struct TransportFilterChain {
    filters: Arc<Vec<Arc<dyn TransportFilter>>>,
}

impl TransportFilterChain {
    /// Creates a new chain from an ordered list of filters.
    pub fn new(filters: Vec<Arc<dyn TransportFilter>>) -> Self {
        Self {
            filters: Arc::new(filters),
        }
    }

    /// Creates an empty chain that accepts everything.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            filters: Arc::new(Vec::new()),
        }
    }

    /// Runs all filters' `on_accept` in order.
    ///
    /// Returns [`FilterVerdict::Reject`] if any filter rejects.
    pub async fn on_accept(&self, ctx: &mut TransportContext) -> FilterVerdict {
        for filter in self.filters.iter() {
            match filter.on_accept(ctx).await {
                FilterVerdict::Continue | FilterVerdict::Modified => continue,
                FilterVerdict::Reject => return FilterVerdict::Reject,
                _ => continue, // non-exhaustive
            }
        }
        FilterVerdict::Continue
    }

    // TODO: on_client_data/on_server_data wrapping
    // These require wrapping the TCP stream to intercept raw bytes.
    // Will be implemented when a real use case demands it.

    /// Notifies all filters of connection close.
    pub fn on_close(&self, ctx: &TransportContext) {
        for filter in self.filters.iter() {
            filter.on_close(ctx);
        }
    }

    /// Returns `true` if the chain has no filters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::net::SocketAddr;
    use std::time::Instant;

    use infrarust_api::event::BoxFuture;
    use infrarust_api::filter::*;
    use infrarust_api::types::Extensions;

    use super::*;

    struct AcceptFilter {
        verdict: FilterVerdict,
    }

    // FilterVerdict doesn't impl Clone, so we need a helper
    fn make_verdict(reject: bool) -> FilterVerdict {
        if reject {
            FilterVerdict::Reject
        } else {
            FilterVerdict::Continue
        }
    }

    impl TransportFilter for AcceptFilter {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata::new("accept_filter")
        }

        fn on_accept<'a>(&'a self, _ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict> {
            let reject = matches!(self.verdict, FilterVerdict::Reject);
            Box::pin(async move { make_verdict(reject) })
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

    fn test_ctx() -> TransportContext {
        TransportContext {
            remote_addr: "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            local_addr: "0.0.0.0:25565".parse::<SocketAddr>().unwrap(),
            real_ip: None,
            connection_time: Instant::now(),
            bytes_received: 0,
            bytes_sent: 0,
            connection_id: 1,
            extensions: Extensions::new(),
        }
    }

    #[tokio::test]
    async fn test_accept_reject() {
        let chain = TransportFilterChain::new(vec![Arc::new(AcceptFilter {
            verdict: FilterVerdict::Reject,
        })]);
        let mut ctx = test_ctx();
        let result = chain.on_accept(&mut ctx).await;
        assert!(matches!(result, FilterVerdict::Reject));
    }

    #[tokio::test]
    async fn test_accept_continue() {
        let chain = TransportFilterChain::new(vec![Arc::new(AcceptFilter {
            verdict: FilterVerdict::Continue,
        })]);
        let mut ctx = test_ctx();
        let result = chain.on_accept(&mut ctx).await;
        assert!(matches!(result, FilterVerdict::Continue));
    }

    #[tokio::test]
    async fn test_chain_order() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_counter = Arc::new(AtomicU32::new(0));

        struct OrderedFilter {
            id: &'static str,
            counter: Arc<AtomicU32>,
            actual_order: Arc<std::sync::Mutex<u32>>,
        }

        impl TransportFilter for OrderedFilter {
            fn metadata(&self) -> FilterMetadata {
                FilterMetadata::new(self.id)
            }

            fn on_accept<'a>(
                &'a self,
                _ctx: &'a mut TransportContext,
            ) -> BoxFuture<'a, FilterVerdict> {
                let order = self.counter.fetch_add(1, Ordering::Relaxed);
                *self.actual_order.lock().unwrap() = order;
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

        let order_a = Arc::new(std::sync::Mutex::new(u32::MAX));
        let order_b = Arc::new(std::sync::Mutex::new(u32::MAX));

        let chain = TransportFilterChain::new(vec![
            Arc::new(OrderedFilter {
                id: "first",
                counter: Arc::clone(&call_counter),
                actual_order: Arc::clone(&order_a),
            }),
            Arc::new(OrderedFilter {
                id: "second",
                counter: Arc::clone(&call_counter),
                actual_order: Arc::clone(&order_b),
            }),
        ]);

        let mut ctx = test_ctx();
        chain.on_accept(&mut ctx).await;

        assert_eq!(*order_a.lock().unwrap(), 0);
        assert_eq!(*order_b.lock().unwrap(), 1);
    }
}
