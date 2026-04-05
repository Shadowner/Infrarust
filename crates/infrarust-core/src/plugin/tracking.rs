//! Tracking wrappers that record registered resources for automatic cleanup.

use std::any::TypeId;
use std::sync::{Arc, Mutex};

use infrarust_api::command::{CommandHandler, CommandManager};
use infrarust_api::event::bus::{ErasedAsyncHandler, ErasedHandler, EventBus};
use infrarust_api::event::{ConnectionState, ListenerHandle, PacketDirection, PacketFilter};
use infrarust_api::services::scheduler::{Scheduler, TaskHandle};

/// Wraps an [`EventBus`] and records all [`ListenerHandle`]s for later cleanup.
pub struct TrackingEventBus {
    inner: Arc<dyn EventBus>,
    handles: Arc<Mutex<Vec<ListenerHandle>>>,
}

impl TrackingEventBus {
    pub fn new(inner: Arc<dyn EventBus>, handles: Arc<Mutex<Vec<ListenerHandle>>>) -> Self {
        Self { inner, handles }
    }

    fn track(&self, handle: ListenerHandle) -> ListenerHandle {
        self.handles.lock().expect("lock poisoned").push(handle);
        handle
    }
}

impl infrarust_api::event::bus::private::Sealed for TrackingEventBus {}

impl EventBus for TrackingEventBus {
    fn subscribe_erased(
        &self,
        event_type: TypeId,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle {
        let handle = self.inner.subscribe_erased(event_type, priority, handler);
        self.track(handle)
    }

    fn subscribe_async_erased(
        &self,
        event_type: TypeId,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        let handle = self
            .inner
            .subscribe_async_erased(event_type, priority, handler);
        self.track(handle)
    }

    fn subscribe_packet(
        &self,
        filter: PacketFilter,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle {
        let handle = self.inner.subscribe_packet(filter, priority, handler);
        self.track(handle)
    }

    fn subscribe_packet_async(
        &self,
        filter: PacketFilter,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        let handle = self.inner.subscribe_packet_async(filter, priority, handler);
        self.track(handle)
    }

    fn has_packet_listeners(
        &self,
        packet_id: i32,
        state: ConnectionState,
        direction: PacketDirection,
    ) -> bool {
        self.inner.has_packet_listeners(packet_id, state, direction)
    }

    fn unsubscribe(&self, handle: ListenerHandle) {
        self.inner.unsubscribe(handle);
    }
}

/// Wraps a [`CommandManager`] and records registered command names for cleanup.
pub struct TrackingCommandManager {
    inner: Arc<dyn CommandManager>,
    commands: Arc<Mutex<Vec<String>>>,
    plugin_id: String,
}

impl TrackingCommandManager {
    pub fn new(
        inner: Arc<dyn CommandManager>,
        commands: Arc<Mutex<Vec<String>>>,
        plugin_id: String,
    ) -> Self {
        Self {
            inner,
            commands,
            plugin_id,
        }
    }
}

impl infrarust_api::command::private::Sealed for TrackingCommandManager {}

impl CommandManager for TrackingCommandManager {
    fn register(
        &self,
        name: &str,
        aliases: &[&str],
        description: &str,
        handler: Box<dyn CommandHandler>,
    ) {
        self.inner
            .register_with_plugin_id(name, aliases, description, handler, &self.plugin_id);
        self.commands
            .lock()
            .expect("lock poisoned")
            .push(name.to_string());
    }

    fn unregister(&self, name: &str) {
        self.inner.unregister(name);
    }
}

/// Wraps a [`Scheduler`] and records [`TaskHandle`]s for cleanup.
pub struct TrackingScheduler {
    inner: Arc<dyn Scheduler>,
    tasks: Arc<Mutex<Vec<TaskHandle>>>,
}

impl TrackingScheduler {
    pub fn new(inner: Arc<dyn Scheduler>, tasks: Arc<Mutex<Vec<TaskHandle>>>) -> Self {
        Self { inner, tasks }
    }
}

impl infrarust_api::services::scheduler::private::Sealed for TrackingScheduler {}

impl Scheduler for TrackingScheduler {
    fn delay(&self, duration: std::time::Duration, task: Box<dyn FnOnce() + Send>) -> TaskHandle {
        let handle = self.inner.delay(duration, task);
        self.tasks.lock().expect("lock poisoned").push(handle);
        handle
    }

    fn interval(
        &self,
        period: std::time::Duration,
        task: Box<dyn Fn() + Send + Sync>,
    ) -> TaskHandle {
        let handle = self.inner.interval(period, task);
        self.tasks.lock().expect("lock poisoned").push(handle);
        handle
    }

    fn interval_with_delay(
        &self,
        period: std::time::Duration,
        delay: std::time::Duration,
        task: Box<dyn Fn() + Send + Sync>,
    ) -> TaskHandle {
        let handle = self.inner.interval_with_delay(period, delay, task);
        self.tasks.lock().expect("lock poisoned").push(handle);
        handle
    }

    fn cancel(&self, handle: TaskHandle) {
        self.inner.cancel(handle);
    }
}
