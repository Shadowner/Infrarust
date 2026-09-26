//! Owner-aware store behind [`CodecFilterRegistry`](infrarust_api::filter::CodecFilterRegistry).

use std::collections::HashMap;

use infrarust_api::filter::{CodecFilterFactory, FilterMetadata, FilterRegistryError};

use super::registry_base::{FilterOwner, FilterRegistryBase, HasFilterMetadata};

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
                .map(|entry| (entry.metadata.id.as_str(), entry.item.as_ref()))
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
                .map(|entry| (entry.metadata.id.as_str(), entry.item.as_ref()))
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

    pub fn register_builtin(
        &self,
        factory: Box<dyn CodecFilterFactory>,
    ) -> Result<(), FilterRegistryError> {
        self.base.register(FilterOwner::Proxy, factory)
    }

    pub fn unregister_builtin(&self, filter_id: &str) -> Result<(), FilterRegistryError> {
        self.base.unregister(&FilterOwner::Proxy, filter_id)
    }

    pub fn register_owned(
        &self,
        plugin_id: &str,
        factory: Box<dyn CodecFilterFactory>,
    ) -> Result<(), FilterRegistryError> {
        self.base.register(FilterOwner::plugin(plugin_id), factory)
    }

    pub fn unregister_owned(
        &self,
        plugin_id: &str,
        filter_id: &str,
    ) -> Result<(), FilterRegistryError> {
        self.base
            .unregister(&FilterOwner::plugin(plugin_id), filter_id)
    }

    pub fn unregister_owner(&self, plugin_id: &str) -> usize {
        self.base.unregister_owner(&FilterOwner::plugin(plugin_id))
    }

    #[must_use]
    pub fn owner_of(&self, filter_id: &str) -> Option<FilterOwner> {
        self.base.owner_of(filter_id)
    }

    #[must_use]
    pub fn owned_by(&self, plugin_id: &str) -> Vec<String> {
        self.base.owned_by(&FilterOwner::plugin(plugin_id))
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

        registry
            .register_builtin(Box::new(MockFactory {
                id: "last_filter",
                priority: FilterPriority::Last,
                create_count: count1,
            }))
            .unwrap();
        registry
            .register_builtin(Box::new(MockFactory {
                id: "first_filter",
                priority: FilterPriority::First,
                create_count: count2,
            }))
            .unwrap();

        let instances = registry.create_instances(&test_init());
        assert_eq!(instances.len(), 2);
    }

    #[test]
    fn test_unregister() {
        let registry = CodecFilterRegistryImpl::new();
        let count = Arc::new(AtomicU32::new(0));

        registry
            .register_builtin(Box::new(MockFactory {
                id: "test_filter",
                priority: FilterPriority::Normal,
                create_count: count,
            }))
            .unwrap();
        assert!(!registry.is_empty());

        registry.unregister_builtin("test_filter").unwrap();
        assert!(registry.is_empty());
    }

    fn counted(id: &'static str) -> (Box<dyn CodecFilterFactory>, Arc<AtomicU32>) {
        let count = Arc::new(AtomicU32::new(0));
        let factory = Box::new(MockFactory {
            id,
            priority: FilterPriority::Normal,
            create_count: Arc::clone(&count),
        });
        (factory, count)
    }

    fn owned_by(id: &str, owner: &str) -> FilterRegistryError {
        FilterRegistryError::OwnedBy {
            id: id.into(),
            owner: owner.into(),
        }
    }

    #[test]
    fn sessions_keep_using_the_owners_factory_after_a_refused_takeover() {
        let registry = CodecFilterRegistryImpl::new();
        let (owned, owner_count) = counted("shared");
        let (stolen, thief_count) = counted("shared");
        let (builtin, builtin_count) = counted("core");
        let (shadow, shadow_count) = counted("core");
        registry.register_owned("owner", owned).unwrap();
        registry.register_builtin(builtin).unwrap();

        assert_eq!(
            registry.register_owned("thief", stolen),
            Err(owned_by("shared", "owner"))
        );
        assert_eq!(
            registry.unregister_owned("thief", "shared"),
            Err(owned_by("shared", "owner"))
        );
        assert_eq!(
            registry.register_owned("thief", shadow),
            Err(owned_by("core", PROXY_FILTER_OWNER))
        );
        assert_eq!(
            registry.unregister_owned("thief", "core"),
            Err(owned_by("core", PROXY_FILTER_OWNER))
        );

        assert_eq!(registry.create_instances(&test_init()).len(), 2);
        assert_eq!(owner_count.load(Ordering::Relaxed), 1);
        assert_eq!(builtin_count.load(Ordering::Relaxed), 1);
        assert_eq!(thief_count.load(Ordering::Relaxed), 0);
        assert_eq!(shadow_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn removing_a_plugins_filters_stops_new_sessions_from_creating_them() {
        let registry = CodecFilterRegistryImpl::new();
        let (first, first_count) = counted("first");
        let (second, second_count) = counted("second");
        let (other, other_count) = counted("other");
        registry.register_owned("gone", first).unwrap();
        registry.register_owned("gone", second).unwrap();
        registry.register_owned("kept", other).unwrap();

        assert_eq!(registry.unregister_owner("gone"), 2);
        assert!(registry.owned_by("gone").is_empty());
        assert_eq!(
            registry.owner_of("other"),
            Some(FilterOwner::plugin("kept"))
        );

        assert_eq!(registry.create_instances(&test_init()).len(), 1);
        assert_eq!(first_count.load(Ordering::Relaxed), 0);
        assert_eq!(second_count.load(Ordering::Relaxed), 0);
        assert_eq!(other_count.load(Ordering::Relaxed), 1);
    }
}
