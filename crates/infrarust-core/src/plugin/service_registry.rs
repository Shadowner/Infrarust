use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock, Weak};

use infrarust_api::error::ServiceError;
use infrarust_api::events::plugin::{ServiceProvidedEvent, ServiceRemovedEvent};
use infrarust_api::services::service_registry::{ErasedService, ServiceHandle, ServiceRegistry};

use crate::event_bus::EventBusImpl;

struct Provided {
    id: u64,
    name: &'static str,
    owner: Arc<str>,
    instance: ErasedService,
}

pub struct ServiceRegistryImpl {
    entries: RwLock<HashMap<TypeId, Provided>>,
    next_id: AtomicU64,
    event_bus: Option<Arc<EventBusImpl>>,
}

impl ServiceRegistryImpl {
    pub fn new(event_bus: Option<Arc<EventBusImpl>>) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            event_bus,
        }
    }

    pub fn provide(
        &self,
        owner: &str,
        service: TypeId,
        name: &'static str,
        instance: ErasedService,
    ) -> Result<u64, ServiceError> {
        let id = {
            let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
            if let Some(existing) = entries.get(&service) {
                return Err(ServiceError::AlreadyProvided {
                    service: existing.name,
                    by: existing.owner.to_string(),
                });
            }
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            entries.insert(
                service,
                Provided {
                    id,
                    name,
                    owner: Arc::from(owner),
                    instance,
                },
            );
            id
        };
        tracing::debug!(plugin = %owner, service = name, "service provided");
        if let Some(bus) = &self.event_bus {
            bus.post(ServiceProvidedEvent::new(service, name, owner));
        }
        Ok(id)
    }

    pub fn withdraw(&self, id: u64) -> bool {
        let removed = {
            let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
            let key = entries
                .iter()
                .find(|(_, provided)| provided.id == id)
                .map(|(key, _)| *key);
            key.and_then(|key| entries.remove(&key).map(|provided| (key, provided)))
        };
        match removed {
            Some((key, provided)) => {
                self.announce_removed(key, &provided);
                true
            }
            None => false,
        }
    }

    pub fn withdraw_owner(&self, owner: &str) -> usize {
        let removed: Vec<(TypeId, Provided)> = {
            let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
            let mut owned: Vec<(TypeId, Provided)> = entries
                .extract_if(|_, provided| &*provided.owner == owner)
                .collect();
            owned.sort_by_key(|(_, provided)| provided.id);
            owned
        };
        for (key, provided) in &removed {
            self.announce_removed(*key, provided);
        }
        removed.len()
    }

    pub fn get(&self, service: TypeId) -> Option<ErasedService> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&service)
            .map(|provided| Arc::clone(&provided.instance))
    }

    pub fn provider(&self, service: TypeId) -> Option<String> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&service)
            .map(|provided| provided.owner.to_string())
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn announce_removed(&self, service: TypeId, provided: &Provided) {
        tracing::debug!(plugin = %provided.owner, service = provided.name, "service withdrawn");
        if let Some(bus) = &self.event_bus {
            bus.post(ServiceRemovedEvent::new(
                service,
                provided.name,
                provided.owner.to_string(),
            ));
        }
    }
}

impl Default for ServiceRegistryImpl {
    fn default() -> Self {
        Self::new(None)
    }
}

pub struct PluginServiceRegistry {
    inner: Arc<ServiceRegistryImpl>,
    owner: Arc<str>,
}

impl PluginServiceRegistry {
    pub fn new(inner: Arc<ServiceRegistryImpl>, plugin_id: &str) -> Self {
        Self {
            inner,
            owner: Arc::from(plugin_id),
        }
    }

    pub fn withdraw_all(&self) -> usize {
        self.inner.withdraw_owner(&self.owner)
    }
}

impl infrarust_api::services::service_registry::private::Sealed for PluginServiceRegistry {}

impl ServiceRegistry for PluginServiceRegistry {
    fn provide_erased(
        &self,
        service: TypeId,
        name: &'static str,
        instance: ErasedService,
    ) -> Result<ServiceHandle, ServiceError> {
        let id = self.inner.provide(&self.owner, service, name, instance)?;
        let registry: Weak<ServiceRegistryImpl> = Arc::downgrade(&self.inner);
        Ok(ServiceHandle::new(name, move || {
            registry
                .upgrade()
                .is_some_and(|registry| registry.withdraw(id))
        }))
    }

    fn get_erased(&self, service: TypeId) -> Option<ErasedService> {
        self.inner.get(service)
    }

    fn provider_erased(&self, service: TypeId) -> Option<String> {
        self.inner.provider(service)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_api::services::service_registry::ServiceRegistryExt;

    use super::*;

    trait IsLoggedIn: Send + Sync {
        fn is_logged_in(&self, username: &str) -> bool;
    }

    struct Everyone;

    impl IsLoggedIn for Everyone {
        fn is_logged_in(&self, _username: &str) -> bool {
            true
        }
    }

    fn pair() -> (
        Arc<ServiceRegistryImpl>,
        PluginServiceRegistry,
        PluginServiceRegistry,
    ) {
        let shared = Arc::new(ServiceRegistryImpl::default());
        let auth = PluginServiceRegistry::new(Arc::clone(&shared), "auth");
        let lobby = PluginServiceRegistry::new(Arc::clone(&shared), "lobby");
        (shared, auth, lobby)
    }

    #[test]
    fn a_consumer_sees_what_another_plugin_provides() {
        let (_, auth, lobby) = pair();
        auth.provide::<dyn IsLoggedIn>(Arc::new(Everyone)).unwrap();
        let service = lobby.get::<dyn IsLoggedIn>().unwrap();
        assert!(service.is_logged_in("Steve"));
        assert_eq!(lobby.provider::<dyn IsLoggedIn>().as_deref(), Some("auth"));
    }

    #[test]
    fn the_first_provider_wins() {
        let (_, auth, lobby) = pair();
        auth.provide::<dyn IsLoggedIn>(Arc::new(Everyone)).unwrap();
        let refused = lobby.provide::<dyn IsLoggedIn>(Arc::new(Everyone));
        assert!(matches!(
            refused,
            Err(ServiceError::AlreadyProvided { by, .. }) if by == "auth"
        ));
        assert!(auth.get::<dyn IsLoggedIn>().is_some());
    }

    #[test]
    fn withdrawing_frees_the_slot_once() {
        let (shared, auth, lobby) = pair();
        let handle = auth.provide::<dyn IsLoggedIn>(Arc::new(Everyone)).unwrap();
        assert!(handle.withdraw());
        assert!(!handle.withdraw());
        assert!(lobby.get::<dyn IsLoggedIn>().is_none());
        lobby.provide::<dyn IsLoggedIn>(Arc::new(Everyone)).unwrap();
        assert!(
            !handle.withdraw(),
            "a stale handle leaves the new provider alone"
        );
        assert_eq!(shared.len(), 1);
    }

    #[test]
    fn withdraw_all_only_drops_the_owner_entries() {
        let (shared, auth, lobby) = pair();
        auth.provide::<dyn IsLoggedIn>(Arc::new(Everyone)).unwrap();
        auth.provide::<String>(Arc::new("x".to_string())).unwrap();
        lobby.provide::<u32>(Arc::new(7)).unwrap();
        assert_eq!(auth.withdraw_all(), 2);
        assert_eq!(shared.len(), 1);
        assert_eq!(*lobby.get::<u32>().unwrap(), 7);
    }
}
