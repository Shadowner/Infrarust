use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

type Listener = Box<dyn Fn(&dyn Any) + Send + Sync>;

pub struct EventBus {
    listeners: Mutex<HashMap<TypeId, Vec<Listener>>>,
}

impl EventBus {
    /// Registers a listener for a specific event type.
    ///
    /// # Example
    /// ```rust
    /// BUS.on::<EVENT_TYPE, _>(|event| {
    ///     println!("Event was emited and found info: {}", event.OBJECT);
    /// });
    /// ```
    pub fn on<E, F>(&self, listener: F)
    where
        E: Any + Send + Sync + 'static,
        F: Fn(&E) + Send + Sync + 'static,
    {
        let mut map = self.listeners.lock().unwrap();
        let entry = map.entry(TypeId::of::<E>()).or_default();

        let wrapper: Listener = Box::new(move |ev| {
            if let Some(e) = ev.downcast_ref::<E>() {
                listener(e);
            }
        });

        entry.push(wrapper);
    }

    /// Emits an event. All listeners registered for this event type will be called.
    ///
    /// # Example
    /// ```rust
    /// // BUS is a global references you can grab from infrarust_event
    /// BUS.emit(&EVENT_TYPE {
    ///     /* fill in fields for your event */
    /// });
    /// ```
    pub fn emit<E>(&self, event: &E)
    where
        E: Any + Send + Sync + 'static,
    {
        let map = self.listeners.lock().unwrap();
        if let Some(listeners) = map.get(&TypeId::of::<E>()) {
            for listener in listeners {
                listener(event);
            }
        }
    }
}

pub static BUS: LazyLock<EventBus> = LazyLock::new(|| EventBus {
    listeners: Mutex::new(HashMap::new()),
});
