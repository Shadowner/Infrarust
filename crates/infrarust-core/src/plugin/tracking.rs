//! Tracking wrappers that record registered resources for automatic cleanup.

use std::any::TypeId;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::command::{
    CommandError, CommandHandler, CommandInfo, CommandManager, CommandRegistration, CommandSpec,
};
use infrarust_api::event::bus::{
    ErasedAsyncHandler, ErasedEvent, ErasedHandler, EventBus, FireError,
};
use infrarust_api::event::{
    BoxFuture, ConnectionState, ListenerHandle, PacketDirection, PacketFilter,
};
use infrarust_api::events::named::NamedEvent;
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
}

impl TrackingEventBus {
    pub fn new(inner: Arc<EventBusImpl>, plugin_id: &str) -> Self {
        Self {
            inner,
            owner: Arc::from(plugin_id),
        }
    }

    pub fn tracked_count(&self) -> usize {
        self.inner.listener_count(&self.owner)
    }

    pub fn unsubscribe_all(&self) {
        self.inner.unsubscribe_owner(&self.owner);
    }
}

impl infrarust_api::__sealed::Sealed for TrackingEventBus {}

impl EventBus for TrackingEventBus {
    fn subscribe_erased(
        &self,
        event_type: TypeId,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle {
        self.inner.subscribe_owned(
            Arc::clone(&self.owner),
            event_type,
            priority,
            HandlerKind::from_sync(handler),
        )
    }

    fn subscribe_async_erased(
        &self,
        event_type: TypeId,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        self.inner.subscribe_owned(
            Arc::clone(&self.owner),
            event_type,
            priority,
            HandlerKind::from_async(handler),
        )
    }

    fn subscribe_packet(
        &self,
        filter: PacketFilter,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle {
        self.inner.subscribe_packet_owned(
            Arc::clone(&self.owner),
            filter,
            priority,
            HandlerKind::from_sync(handler),
        )
    }

    fn subscribe_packet_async(
        &self,
        filter: PacketFilter,
        priority: infrarust_api::event::EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        self.inner.subscribe_packet_owned(
            Arc::clone(&self.owner),
            filter,
            priority,
            HandlerKind::from_async(handler),
        )
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
        let removed = self.inner.unsubscribe_owned(&self.owner, handle);
        if !removed {
            tracing::warn!(
                plugin = %self.owner,
                listener = handle.as_u64(),
                "refused to unsubscribe a listener this plugin did not register"
            );
        }
        removed
    }

    fn fire_erased<'a>(
        &'a self,
        event: &'a mut dyn ErasedEvent,
    ) -> BoxFuture<'a, Result<(), FireError>> {
        if let Some(named) = event.as_any_mut().downcast_mut::<NamedEvent>() {
            named.source_plugin = self.owner.to_string();
        }
        Box::pin(self.inner.fire_from(&self.owner, event))
    }
}

pub struct TrackingCommandManager {
    inner: Arc<CommandManagerImpl>,
    plugin_id: String,
}

impl TrackingCommandManager {
    pub fn new(inner: Arc<CommandManagerImpl>, plugin_id: String) -> Self {
        Self { inner, plugin_id }
    }

    pub fn tracked(&self) -> Vec<String> {
        self.inner.keys_owned_by(&self.plugin_id)
    }

    pub fn unregister_all(&self) {
        self.inner.unregister_owner(&self.plugin_id);
    }
}

impl infrarust_api::__sealed::Sealed for TrackingCommandManager {}

impl CommandManager for TrackingCommandManager {
    fn register(
        &self,
        spec: CommandSpec,
        handler: Box<dyn CommandHandler>,
    ) -> Result<CommandRegistration, CommandError> {
        self.inner.register_owned(&self.plugin_id, spec, handler)
    }

    fn unregister(&self, name: &str) -> Result<(), CommandError> {
        self.inner.unregister_owned(&self.plugin_id, name).map(drop)
    }

    fn get(&self, label: &str) -> Option<CommandInfo> {
        self.inner.get(label)
    }

    fn get_by_name(&self, name: &str) -> Option<CommandInfo> {
        self.inner.get_by_name(name)
    }

    fn get_by_alias(&self, alias: &str) -> Option<CommandInfo> {
        self.inner.get_by_alias(alias)
    }

    fn contains(&self, label: &str) -> bool {
        self.inner.contains(label)
    }

    fn list(&self) -> Vec<CommandInfo> {
        self.inner.list()
    }

    fn list_owned(&self) -> Vec<CommandInfo> {
        self.inner.commands_for_plugin(&self.plugin_id)
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

impl infrarust_api::__sealed::Sealed for TrackingCodecFilterRegistry {}

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

impl infrarust_api::__sealed::Sealed for TrackingTransportFilterRegistry {}

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

impl infrarust_api::__sealed::Sealed for TrackingScheduler {}

impl Scheduler for TrackingScheduler {
    fn delay(&self, duration: Duration, task: AsyncTask) -> TaskHandle {
        self.inner.delay_for(&self.owner, duration, task)
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

    fn spawn(&self, task: BoxFuture<'static, ()>) -> TaskHandle {
        self.inner.spawn_for(&self.owner, task)
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
