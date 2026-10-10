use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{Event, EventPriority, ListenerHandle};

use super::{EventVisitor, WasmEvent, kind_name, visit_kind};
use crate::actor::InstanceRef;
use crate::bindings::infrarust::plugin::events::EventKind;

struct Subscribed {
    kind: EventKind,
    priority: u8,
    count: usize,
}

pub(crate) struct AccessListeners {
    serving: AtomicBool,
    subscribed: Mutex<Vec<Subscribed>>,
}

impl Default for AccessListeners {
    fn default() -> Self {
        Self {
            serving: AtomicBool::new(true),
            subscribed: Mutex::new(Vec::new()),
        }
    }
}

impl AccessListeners {
    pub(crate) fn serving(&self) -> bool {
        self.serving.load(Ordering::Acquire)
    }

    pub(crate) fn set_serving(&self, serving: bool) {
        self.serving.store(serving, Ordering::Release);
    }

    pub(crate) fn subscribed(&self) -> Vec<(EventKind, EventPriority)> {
        self.lock()
            .iter()
            .map(|entry| (entry.kind, EventPriority::custom(entry.priority)))
            .collect()
    }

    pub(crate) fn track(self: &Arc<Self>, kind: EventKind, priority: EventPriority) -> Tracked {
        let priority = priority.value();
        let mut subscribed = self.lock();
        match subscribed
            .iter_mut()
            .find(|entry| entry.kind == kind && entry.priority == priority)
        {
            Some(entry) => entry.count += 1,
            None => subscribed.push(Subscribed {
                kind,
                priority,
                count: 1,
            }),
        }
        Tracked {
            listeners: Arc::clone(self),
            kind,
            priority,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Subscribed>> {
        self.subscribed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct Tracked {
    listeners: Arc<AccessListeners>,
    kind: EventKind,
    priority: u8,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        let mut subscribed = self.listeners.lock();
        if let Some(entry) = subscribed
            .iter_mut()
            .find(|entry| entry.kind == self.kind && entry.priority == self.priority)
        {
            entry.count = entry.count.saturating_sub(1);
        }
        subscribed.retain(|entry| entry.count > 0);
    }
}

struct Guard<'b> {
    bus: &'b dyn EventBus,
    instance: InstanceRef,
    priority: EventPriority,
}

impl EventVisitor for Guard<'_> {
    type Output = Option<ListenerHandle>;

    fn visit<E: WasmEvent + Event>(self) -> Option<ListenerHandle> {
        let deny = E::DENY_UNANSWERED?;
        let instance = self.instance;
        Some(
            self.bus
                .subscribe::<E, _>(self.priority, move |event: &mut E| {
                    if instance.access().serving() {
                        return;
                    }
                    deny(event);
                    if let Some(suppressed) = instance.admit_warning() {
                        tracing::warn!(
                            plugin = instance.plugin_id(),
                            event = kind_name(E::KIND),
                            suppressed,
                            "access event denied: the wasm plugin listening to it has no live instance \
                             while it recovers from a fault or is quarantined"
                        );
                    }
                }),
        )
    }
}

pub(crate) fn guard(
    bus: &dyn EventBus,
    instance: InstanceRef,
    kind: EventKind,
    priority: EventPriority,
) -> Option<ListenerHandle> {
    let guard = Guard {
        bus,
        instance,
        priority,
    };
    visit_kind(kind, guard).flatten()
}
