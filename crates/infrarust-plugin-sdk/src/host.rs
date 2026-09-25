pub(crate) use imp::*;

#[cfg(target_family = "wasm")]
mod imp {
    use crate::bindings::codec_registry::{self, CodecFilterMetadata};
    use crate::bindings::event_bus::{self, EventKind};
    use crate::bindings::{command_manager, limbo, scheduler};

    pub(crate) fn subscribe(kind: EventKind, priority: u8) -> u64 {
        event_bus::subscribe(kind, priority)
    }

    pub(crate) fn unsubscribe(listener: u64) {
        event_bus::unsubscribe(listener);
    }

    pub(crate) fn register_command(
        name: &str,
        aliases: &[String],
        description: &str,
        callback_id: u64,
    ) {
        command_manager::register(name, aliases, description, callback_id);
    }

    pub(crate) fn unregister_command(name: &str) {
        command_manager::unregister(name);
    }

    pub(crate) fn delay(after_ms: u64, callback_id: u64) -> u64 {
        scheduler::delay(after_ms, callback_id)
    }

    pub(crate) fn interval(period_ms: u64, callback_id: u64) -> u64 {
        scheduler::interval(period_ms, callback_id)
    }

    pub(crate) fn cancel(handle: u64) {
        scheduler::cancel(handle);
    }

    pub(crate) fn register_codec_filter(metadata: &CodecFilterMetadata, factory: u64) {
        codec_registry::register_codec_filter(metadata, factory);
    }

    pub(crate) fn register_limbo_handler(name: &str, handler: u64) {
        limbo::register_limbo_handler(name, handler);
    }
}

#[cfg(not(target_family = "wasm"))]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use crate::bindings::codec_registry::CodecFilterMetadata;
    use crate::bindings::event_bus::EventKind;

    #[derive(Default)]
    pub(crate) struct FakeHost {
        next_handle: u64,
        pub(crate) listeners: HashMap<u64, EventKind>,
        pub(crate) commands: HashMap<String, u64>,
        pub(crate) tasks: HashMap<u64, (u64, bool)>,
        pub(crate) cancelled: Vec<u64>,
        pub(crate) codec_filters: Vec<(String, u64)>,
        pub(crate) limbo_handlers: Vec<(String, u64)>,
    }

    impl FakeHost {
        fn next_handle(&mut self) -> u64 {
            self.next_handle += 1;
            self.next_handle
        }

        fn schedule(&mut self, callback_id: u64, repeating: bool) -> u64 {
            let handle = self.next_handle();
            self.tasks.insert(handle, (callback_id, repeating));
            handle
        }
    }

    thread_local! {
        static FAKE: RefCell<FakeHost> = RefCell::new(FakeHost::default());
    }

    pub(crate) fn with_fake<R>(f: impl FnOnce(&mut FakeHost) -> R) -> R {
        FAKE.with(|host| f(&mut host.borrow_mut()))
    }

    pub(crate) fn subscribe(kind: EventKind, _priority: u8) -> u64 {
        with_fake(|host| {
            let handle = host.next_handle();
            host.listeners.insert(handle, kind);
            handle
        })
    }

    pub(crate) fn unsubscribe(listener: u64) {
        with_fake(|host| host.listeners.remove(&listener));
    }

    pub(crate) fn register_command(
        name: &str,
        _aliases: &[String],
        _description: &str,
        callback_id: u64,
    ) {
        with_fake(|host| host.commands.insert(name.to_lowercase(), callback_id));
    }

    pub(crate) fn unregister_command(name: &str) {
        with_fake(|host| host.commands.remove(&name.to_lowercase()));
    }

    pub(crate) fn delay(_after_ms: u64, callback_id: u64) -> u64 {
        with_fake(|host| host.schedule(callback_id, false))
    }

    pub(crate) fn interval(_period_ms: u64, callback_id: u64) -> u64 {
        with_fake(|host| host.schedule(callback_id, true))
    }

    pub(crate) fn cancel(handle: u64) {
        with_fake(|host| {
            host.tasks.remove(&handle);
            host.cancelled.push(handle);
        });
    }

    pub(crate) fn register_codec_filter(metadata: &CodecFilterMetadata, factory: u64) {
        with_fake(|host| host.codec_filters.push((metadata.id.clone(), factory)));
    }

    pub(crate) fn register_limbo_handler(name: &str, handler: u64) {
        with_fake(|host| host.limbo_handlers.push((name.to_owned(), handler)));
    }
}
