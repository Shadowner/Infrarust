//! Tracking wrappers that record registered resources for automatic cleanup.

use std::any::{Any, TypeId};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::command::{
    CommandError, CommandHandler, CommandInfo, CommandManager, CommandRegistration, CommandSpec,
};
use infrarust_api::event::bus::{ErasedAsyncHandler, ErasedHandler, EventBus, FireError};
use infrarust_api::event::{
    BoxFuture, ConnectionState, ListenerHandle, PacketDirection, PacketFilter,
};
use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterRegistry, FilterRegistryError, TransportFilter,
    TransportFilterRegistry,
};
use infrarust_api::services::scheduler::{AsyncTask, RepeatingTask, Scheduler, TaskHandle};

use crate::event_bus::EventBusImpl;
use crate::event_bus::handler::HandlerKind;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::services::command_manager::CommandManagerImpl;
use crate::services::scheduler::SchedulerImpl;

/// Wraps an [`EventBus`] and records all [`ListenerHandle`]s for later cleanup.
pub struct TrackingEventBus {
    inner: Arc<EventBusImpl>,
    owner: Arc<str>,
    handles: Mutex<HashSet<ListenerHandle>>,
}

impl TrackingEventBus {
    pub fn new(inner: Arc<EventBusImpl>, plugin_id: &str) -> Self {
        Self {
            inner,
            owner: Arc::from(plugin_id),
            handles: Mutex::new(HashSet::new()),
        }
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn tracked_count(&self) -> usize {
        self.handles.lock().expect("lock poisoned").len()
    }

    pub fn unsubscribe_all(&self) {
        let handles = std::mem::take(&mut *self.handles.lock().expect("lock poisoned"));
        for handle in handles {
            self.inner.unsubscribe_owned(&self.owner, handle);
        }
    }

    fn track(&self, handle: ListenerHandle) -> ListenerHandle {
        self.handles.lock().expect("lock poisoned").insert(handle);
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
        let handle = self.inner.subscribe_owned(
            Arc::clone(&self.owner),
            event_type,
            priority,
            HandlerKind::from_sync(handler),
        );
        self.track(handle)
    }

    fn subscribe_async_erased(
        &self,
        event_type: TypeId,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        let handle = self.inner.subscribe_owned(
            Arc::clone(&self.owner),
            event_type,
            priority,
            HandlerKind::from_async(handler),
        );
        self.track(handle)
    }

    fn subscribe_packet(
        &self,
        filter: PacketFilter,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle {
        let handle = self.inner.subscribe_packet_owned(
            Arc::clone(&self.owner),
            filter,
            priority,
            HandlerKind::from_sync(handler),
        );
        self.track(handle)
    }

    fn subscribe_packet_async(
        &self,
        filter: PacketFilter,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        let handle = self.inner.subscribe_packet_owned(
            Arc::clone(&self.owner),
            filter,
            priority,
            HandlerKind::from_async(handler),
        );
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

    fn unsubscribe(&self, handle: ListenerHandle) -> bool {
        let tracked = self.handles.lock().expect("lock poisoned").remove(&handle);
        if !tracked {
            tracing::warn!(
                plugin = %self.owner,
                listener = handle.as_u64(),
                "refused to unsubscribe a listener this plugin did not register"
            );
            return false;
        }
        self.inner.unsubscribe_owned(&self.owner, handle)
    }

    fn fire_erased<'a>(
        &'a self,
        event_type: &'static str,
        event: &'a mut (dyn Any + Send),
    ) -> BoxFuture<'a, Result<(), FireError>> {
        Box::pin(self.inner.fire_from(&self.owner, event_type, event))
    }
}

pub struct TrackingCommandManager {
    inner: Arc<CommandManagerImpl>,
    commands: Mutex<Vec<String>>,
    plugin_id: String,
}

impl TrackingCommandManager {
    pub fn new(inner: Arc<CommandManagerImpl>, plugin_id: String) -> Self {
        Self {
            inner,
            commands: Mutex::new(Vec::new()),
            plugin_id,
        }
    }

    pub fn tracked(&self) -> Vec<String> {
        self.commands.lock().expect("lock poisoned").clone()
    }

    pub fn unregister_all(&self) {
        let commands = std::mem::take(&mut *self.commands.lock().expect("lock poisoned"));
        for key in commands {
            if let Err(e) = self.inner.unregister_owned(&self.plugin_id, &key) {
                tracing::debug!(plugin = %self.plugin_id, "command already gone at cleanup: {e}");
            }
        }
    }
}

impl infrarust_api::command::private::Sealed for TrackingCommandManager {}

impl CommandManager for TrackingCommandManager {
    fn register(
        &self,
        spec: CommandSpec,
        handler: Box<dyn CommandHandler>,
    ) -> Result<CommandRegistration, CommandError> {
        let registration = self.inner.register_owned(&self.plugin_id, spec, handler)?;
        let mut commands = self.commands.lock().expect("lock poisoned");
        if !commands.contains(&registration.namespaced) {
            commands.push(registration.namespaced.clone());
        }
        Ok(registration)
    }

    fn unregister(&self, name: &str) -> Result<(), CommandError> {
        let key = self.inner.unregister_owned(&self.plugin_id, name)?;
        self.commands
            .lock()
            .expect("lock poisoned")
            .retain(|tracked| *tracked != key);
        Ok(())
    }

    fn list(&self) -> Vec<CommandInfo> {
        self.inner.list()
    }
}

pub struct TrackingCodecFilterRegistry {
    inner: Arc<CodecFilterRegistryImpl>,
    plugin_id: String,
}

impl TrackingCodecFilterRegistry {
    pub fn new(inner: Arc<CodecFilterRegistryImpl>, plugin_id: String) -> Self {
        Self { inner, plugin_id }
    }

    pub fn unregister_all(&self) -> usize {
        self.inner.unregister_owner(&self.plugin_id)
    }
}

impl infrarust_api::filter::registry::private::Sealed for TrackingCodecFilterRegistry {}

impl CodecFilterRegistry for TrackingCodecFilterRegistry {
    fn register(&self, factory: Box<dyn CodecFilterFactory>) -> Result<(), FilterRegistryError> {
        self.inner.register_owned(&self.plugin_id, factory)
    }

    fn unregister(&self, filter_id: &str) -> Result<(), FilterRegistryError> {
        self.inner.unregister_owned(&self.plugin_id, filter_id)
    }
}

pub struct TrackingTransportFilterRegistry {
    inner: Arc<TransportFilterRegistryImpl>,
    plugin_id: String,
}

impl TrackingTransportFilterRegistry {
    pub fn new(inner: Arc<TransportFilterRegistryImpl>, plugin_id: String) -> Self {
        Self { inner, plugin_id }
    }

    pub fn unregister_all(&self) -> usize {
        self.inner.unregister_owner(&self.plugin_id)
    }
}

impl infrarust_api::filter::registry::private::Sealed for TrackingTransportFilterRegistry {}

impl TransportFilterRegistry for TrackingTransportFilterRegistry {
    fn register(&self, filter: Box<dyn TransportFilter>) -> Result<(), FilterRegistryError> {
        self.inner.register_owned(&self.plugin_id, filter)
    }

    fn unregister(&self, filter_id: &str) -> Result<(), FilterRegistryError> {
        self.inner.unregister_owned(&self.plugin_id, filter_id)
    }
}

pub struct TrackingScheduler {
    inner: Arc<SchedulerImpl>,
    owner: Arc<str>,
}

impl TrackingScheduler {
    pub fn new(inner: Arc<SchedulerImpl>, plugin_id: &str) -> Self {
        Self {
            inner,
            owner: Arc::from(plugin_id),
        }
    }

    pub fn tracked_count(&self) -> usize {
        self.inner.owned_count(&self.owner)
    }

    pub fn cancel_all(&self) -> usize {
        self.inner.cancel_owner(&self.owner)
    }
}

impl infrarust_api::services::scheduler::private::Sealed for TrackingScheduler {}

impl Scheduler for TrackingScheduler {
    fn delay(&self, duration: Duration, task: Box<dyn FnOnce() + Send>) -> TaskHandle {
        self.inner.delay_for(&self.owner, duration, task)
    }

    fn interval(&self, period: Duration, task: Box<dyn Fn() + Send + Sync>) -> TaskHandle {
        self.inner.interval_for(&self.owner, period, period, task)
    }

    fn interval_with_delay(
        &self,
        period: Duration,
        delay: Duration,
        task: Box<dyn Fn() + Send + Sync>,
    ) -> TaskHandle {
        self.inner.interval_for(&self.owner, period, delay, task)
    }

    fn spawn(&self, task: BoxFuture<'static, ()>) -> TaskHandle {
        self.inner.spawn_for(&self.owner, task)
    }

    fn delay_async(&self, duration: Duration, task: AsyncTask) -> TaskHandle {
        self.inner.delay_async_for(&self.owner, duration, task)
    }

    fn repeat(
        &self,
        period: Duration,
        initial_delay: Option<Duration>,
        task: RepeatingTask,
    ) -> TaskHandle {
        self.inner
            .repeat_for(&self.owner, period, initial_delay, task)
    }

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send>) -> TaskHandle {
        self.inner.spawn_blocking_for(&self.owner, task)
    }

    fn cancel(&self, handle: TaskHandle) {
        if !self.inner.cancel_owned(&self.owner, handle) {
            tracing::debug!(
                plugin = %self.owner,
                task = handle.as_u64(),
                "cancel ignored: the task finished or belongs to another plugin"
            );
        }
    }
}
