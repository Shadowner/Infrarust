//! Concrete implementation of [`CodecFilterRegistry`].

use std::collections::HashMap;

use infrarust_api::filter::{CodecFilterFactory, CodecFilterRegistry, FilterMetadata};

use super::registry_base::{FilterRegistryBase, HasFilterMetadata};

impl HasFilterMetadata for Box<dyn CodecFilterFactory> {
    fn metadata(&self) -> FilterMetadata {
        CodecFilterFactory::metadata(self.as_ref())
    }
}

/// Stores registered [`CodecFilterFactory`] instances and maintains
/// a resolved execution order.
///
/// The order is recalculated on each `register`/`unregister` call,
/// not on every packet.
pub struct CodecFilterRegistryImpl {
    base: FilterRegistryBase<Box<dyn CodecFilterFactory>>,
}

impl CodecFilterRegistryImpl {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: FilterRegistryBase::new("codec"),
        }
    }

    /// Creates filter instances from all registered factories in resolved order.
    ///
    /// Called once per session at setup time, not per-packet.
    pub fn create_instances(
        &self,
        init: &infrarust_api::filter::CodecSessionInit,
    ) -> Vec<Box<dyn infrarust_api::filter::CodecFilterInstance>> {
        self.base.with_ordered(|factories, ordered| {
            let factory_map: HashMap<&str, &dyn CodecFilterFactory> = factories
                .iter()
                .map(|(m, f)| (m.id.as_str(), f.as_ref()))
                .collect();

            ordered
                .iter()
                .filter_map(|id| factory_map.get(id.as_str()))
                .map(|f| f.create(init))
                .collect()
        })
    }

    /// Like [`create_instances`](Self::create_instances), but also returns each
    /// instance's resolved filter id (in execution order) for per-filter timing
    /// attribution. Only built under the `bench-timing` feature.
    #[cfg(feature = "bench-timing")]
    pub fn create_instances_with_ids(
        &self,
        init: &infrarust_api::filter::CodecSessionInit,
    ) -> (
        Vec<Box<dyn infrarust_api::filter::CodecFilterInstance>>,
        Vec<std::sync::Arc<str>>,
    ) {
        self.base.with_ordered(|factories, ordered| {
            let factory_map: HashMap<&str, &dyn CodecFilterFactory> = factories
                .iter()
                .map(|(m, f)| (m.id.as_str(), f.as_ref()))
                .collect();

            let mut instances = Vec::with_capacity(ordered.len());
            let mut ids = Vec::with_capacity(ordered.len());
            for id in ordered {
                if let Some(f) = factory_map.get(id.as_str()) {
                    instances.push(f.create(init));
                    ids.push(std::sync::Arc::from(id.as_str()));
                }
            }
            (instances, ids)
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }
}

impl Default for CodecFilterRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl infrarust_api::filter::registry::private::Sealed for CodecFilterRegistryImpl {}

impl CodecFilterRegistry for CodecFilterRegistryImpl {
    fn register(&self, factory: Box<dyn CodecFilterFactory>) {
        self.base.register(factory);
    }

    fn unregister(&self, filter_id: &str) {
        self.base.unregister(filter_id);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use infrarust_api::filter::*;
    use infrarust_api::types::RawPacket;

    use super::*;

    struct MockFactory {
        id: &'static str,
        priority: FilterPriority,
        create_count: Arc<AtomicU32>,
    }

    struct MockInstance;

    impl CodecFilterFactory for MockFactory {
        fn metadata(&self) -> FilterMetadata {
            FilterMetadata {
                id: self.id.to_string(),
                priority: self.priority,
                after: vec![],
                before: vec![],
            }
        }

        fn create(&self, _ctx: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
            self.create_count.fetch_add(1, Ordering::Relaxed);
            Box::new(MockInstance)
        }
    }

    impl CodecFilterInstance for MockInstance {
        fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
            CodecVerdict::Pass
        }
    }

    fn test_init() -> CodecSessionInit {
        CodecSessionInit {
            client_version: infrarust_api::types::ProtocolVersion::new(767),
            connection_id: 1,
            remote_addr: "127.0.0.1:12345".parse().unwrap(),
            real_ip: None,
            side: ConnectionSide::ClientSide,
        }
    }

    #[test]
    fn test_register_and_get_ordered() {
        let registry = CodecFilterRegistryImpl::new();
        let count1 = Arc::new(AtomicU32::new(0));
        let count2 = Arc::new(AtomicU32::new(0));

        registry.register(Box::new(MockFactory {
            id: "last_filter",
            priority: FilterPriority::Last,
            create_count: count1,
        }));
        registry.register(Box::new(MockFactory {
            id: "first_filter",
            priority: FilterPriority::First,
            create_count: count2,
        }));

        let instances = registry.create_instances(&test_init());
        assert_eq!(instances.len(), 2);
    }

    #[test]
    fn test_unregister() {
        let registry = CodecFilterRegistryImpl::new();
        let count = Arc::new(AtomicU32::new(0));

        registry.register(Box::new(MockFactory {
            id: "test_filter",
            priority: FilterPriority::Normal,
            create_count: count,
        }));
        assert!(!registry.is_empty());

        registry.unregister("test_filter");
        assert!(registry.is_empty());
    }
}
