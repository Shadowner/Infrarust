use std::any::{Any, TypeId, type_name};
use std::sync::Arc;

use crate::error::ServiceError;

pub mod private {
    pub trait Sealed {}
}

pub type ErasedService = Arc<dyn Any + Send + Sync>;

#[derive(Clone)]
pub struct ServiceHandle {
    service: &'static str,
    revoke: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl ServiceHandle {
    pub fn new(service: &'static str, revoke: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            service,
            revoke: Arc::new(revoke),
        }
    }

    pub const fn service(&self) -> &'static str {
        self.service
    }

    pub fn withdraw(&self) -> bool {
        (self.revoke)()
    }
}

impl std::fmt::Debug for ServiceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceHandle")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

pub trait ServiceRegistry: Send + Sync + private::Sealed {
    fn provide_erased(
        &self,
        service: TypeId,
        name: &'static str,
        instance: ErasedService,
    ) -> Result<ServiceHandle, ServiceError>;

    fn get_erased(&self, service: TypeId) -> Option<ErasedService>;

    fn provider_erased(&self, service: TypeId) -> Option<String>;
}

pub trait ServiceRegistryExt {
    fn provide<T: ?Sized + Send + Sync + 'static>(
        &self,
        service: Arc<T>,
    ) -> Result<ServiceHandle, ServiceError>;

    fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>>;

    fn provider<T: ?Sized + 'static>(&self) -> Option<String>;
}

impl<R: ServiceRegistry + ?Sized> ServiceRegistryExt for R {
    fn provide<T: ?Sized + Send + Sync + 'static>(
        &self,
        service: Arc<T>,
    ) -> Result<ServiceHandle, ServiceError> {
        self.provide_erased(TypeId::of::<T>(), type_name::<T>(), Arc::new(service))
    }

    fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.get_erased(TypeId::of::<T>())?
            .downcast_ref::<Arc<T>>()
            .cloned()
    }

    fn provider<T: ?Sized + 'static>(&self) -> Option<String> {
        self.provider_erased(TypeId::of::<T>())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    trait Greeter: Send + Sync {
        fn greet(&self) -> String;
    }

    struct English;

    impl Greeter for English {
        fn greet(&self) -> String {
            "hello".into()
        }
    }

    #[derive(Default)]
    struct MapRegistry {
        entries: Mutex<HashMap<TypeId, (&'static str, ErasedService)>>,
    }

    impl private::Sealed for MapRegistry {}

    impl ServiceRegistry for MapRegistry {
        fn provide_erased(
            &self,
            service: TypeId,
            name: &'static str,
            instance: ErasedService,
        ) -> Result<ServiceHandle, ServiceError> {
            let mut entries = self.entries.lock().unwrap();
            if entries.contains_key(&service) {
                return Err(ServiceError::AlreadyProvided {
                    service: name,
                    by: "someone".into(),
                });
            }
            entries.insert(service, (name, instance));
            Ok(ServiceHandle::new(name, || true))
        }

        fn get_erased(&self, service: TypeId) -> Option<ErasedService> {
            self.entries
                .lock()
                .unwrap()
                .get(&service)
                .map(|(_, instance)| Arc::clone(instance))
        }

        fn provider_erased(&self, service: TypeId) -> Option<String> {
            self.entries
                .lock()
                .unwrap()
                .contains_key(&service)
                .then(|| "someone".to_string())
        }
    }

    #[test]
    fn a_trait_object_round_trips_through_the_erased_registry() {
        let registry = MapRegistry::default();
        let handle = registry.provide::<dyn Greeter>(Arc::new(English)).unwrap();
        assert!(handle.service().contains("Greeter"));

        let greeter = registry.get::<dyn Greeter>().unwrap();
        assert_eq!(greeter.greet(), "hello");
        assert!(registry.get::<English>().is_none());
        assert_eq!(
            registry.provider::<dyn Greeter>().as_deref(),
            Some("someone")
        );
    }

    #[test]
    fn a_second_provider_is_refused() {
        let registry: Box<dyn ServiceRegistry> = Box::new(MapRegistry::default());
        registry.provide::<dyn Greeter>(Arc::new(English)).unwrap();
        let refused = registry.provide::<dyn Greeter>(Arc::new(English));
        assert!(matches!(
            refused,
            Err(ServiceError::AlreadyProvided { by, .. }) if by == "someone"
        ));
    }
}
