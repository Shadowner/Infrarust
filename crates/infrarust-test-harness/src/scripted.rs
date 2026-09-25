use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, Event, EventPriority};
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};

type Registration = Box<dyn Fn(&dyn EventBus) + Send + Sync>;
type EnableHook = Box<dyn Fn(&dyn PluginContext) + Send + Sync>;

pub struct ScriptedPlugin {
    id: String,
    registrations: Vec<Registration>,
    enable_hooks: Vec<EnableHook>,
}

impl ScriptedPlugin {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            registrations: Vec::new(),
            enable_hooks: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn on<E: Event>(
        mut self,
        priority: EventPriority,
        handler: impl Fn(&mut E) + Send + Sync + 'static,
    ) -> Self {
        let handler = Arc::new(handler);
        self.registrations.push(Box::new(move |bus: &dyn EventBus| {
            let handler = Arc::clone(&handler);
            bus.subscribe(priority, move |event: &mut E| handler(event));
        }));
        self
    }

    #[must_use]
    pub fn on_async<E: Event>(
        mut self,
        priority: EventPriority,
        handler: impl Fn(&mut E) -> BoxFuture<'_, ()> + Send + Sync + 'static,
    ) -> Self {
        let handler = Arc::new(handler);
        self.registrations.push(Box::new(move |bus: &dyn EventBus| {
            let handler = Arc::clone(&handler);
            bus.subscribe_async::<E, _>(priority, move |event| handler(event));
        }));
        self
    }

    #[must_use]
    pub fn on_enable(mut self, hook: impl Fn(&dyn PluginContext) + Send + Sync + 'static) -> Self {
        self.enable_hooks.push(Box::new(hook));
        self
    }
}

impl Plugin for ScriptedPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new(self.id.clone(), self.id.clone(), "0.0.0")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        for register in &self.registrations {
            register(ctx.event_bus());
        }
        for hook in &self.enable_hooks {
            hook(ctx);
        }
        Box::pin(async { Ok(()) })
    }
}
