use std::any::TypeId;

use crate::event::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginEnabledEvent {
    pub plugin_id: String,
    pub version: String,
}

impl PluginEnabledEvent {
    pub fn new(plugin_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            version: version.into(),
        }
    }
}

impl Event for PluginEnabledEvent {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginDisabledEvent {
    pub plugin_id: String,
}

impl PluginDisabledEvent {
    pub fn new(plugin_id: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
        }
    }
}

impl Event for PluginDisabledEvent {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServiceProvidedEvent {
    pub service: &'static str,
    pub provider: String,
    service_type: TypeId,
}

impl ServiceProvidedEvent {
    pub fn new(service_type: TypeId, service: &'static str, provider: impl Into<String>) -> Self {
        Self {
            service,
            provider: provider.into(),
            service_type,
        }
    }

    pub fn is<T: ?Sized + 'static>(&self) -> bool {
        self.service_type == TypeId::of::<T>()
    }
}

impl Event for ServiceProvidedEvent {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServiceRemovedEvent {
    pub service: &'static str,
    pub provider: String,
    service_type: TypeId,
}

impl ServiceRemovedEvent {
    pub fn new(service_type: TypeId, service: &'static str, provider: impl Into<String>) -> Self {
        Self {
            service,
            provider: provider.into(),
            service_type,
        }
    }

    pub fn is<T: ?Sized + 'static>(&self) -> bool {
        self.service_type == TypeId::of::<T>()
    }
}

impl Event for ServiceRemovedEvent {}

#[cfg(test)]
mod tests {
    use super::*;

    trait Greeter {}

    #[test]
    fn service_events_match_the_type_they_were_built_for() {
        let provided = ServiceProvidedEvent::new(TypeId::of::<dyn Greeter>(), "dyn Greeter", "a");
        assert!(provided.is::<dyn Greeter>());
        assert!(!provided.is::<String>());
        let removed = ServiceRemovedEvent::new(TypeId::of::<dyn Greeter>(), "dyn Greeter", "a");
        assert!(removed.is::<dyn Greeter>());
        assert_eq!(removed.provider, "a");
    }
}
