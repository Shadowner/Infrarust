pub(crate) use imp::*;

#[cfg(target_family = "wasm")]
mod imp {
    use crate::bindings::codec_registry::{self, CodecFilterMetadata};
    use crate::bindings::command_manager::{self, CommandRegistration, CommandSpec};
    use crate::bindings::events::EventKind;
    use crate::bindings::types::HostError;
    use crate::bindings::{event_bus, limbo, scheduler};

    pub(crate) fn subscribe(kind: EventKind, priority: u8) -> Result<u64, HostError> {
        event_bus::subscribe(kind, priority)
    }

    pub(crate) fn unsubscribe(listener: u64) -> Result<bool, HostError> {
        event_bus::unsubscribe(listener)
    }

    pub(crate) fn register_command(
        spec: &CommandSpec,
        handler: u64,
    ) -> Result<CommandRegistration, HostError> {
        command_manager::register(spec, handler)
    }

    pub(crate) fn unregister_command(name: &str) -> Result<(), HostError> {
        command_manager::unregister(name)
    }

    pub(crate) fn delay(after_ms: u64, handler: u64) -> Result<u64, HostError> {
        scheduler::delay(after_ms, handler)
    }

    pub(crate) fn interval(
        period_ms: u64,
        initial_delay_ms: Option<u64>,
        handler: u64,
    ) -> Result<u64, HostError> {
        scheduler::interval(period_ms, initial_delay_ms, handler)
    }

    pub(crate) fn cancel(handle: u64) -> Result<(), HostError> {
        scheduler::cancel(handle)
    }

    pub(crate) fn register_codec_filter(
        metadata: &CodecFilterMetadata,
        factory: u64,
    ) -> Result<(), HostError> {
        codec_registry::register_codec_filter(metadata, factory)
    }

    pub(crate) fn register_limbo_handler(name: &str, handler: u64) -> Result<(), HostError> {
        limbo::register_limbo_handler(name, handler)
    }
}

#[cfg(not(target_family = "wasm"))]
mod imp {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    use crate::bindings::codec_registry::CodecFilterMetadata;
    use crate::bindings::command_manager::{CommandRegistration, CommandSpec};
    use crate::bindings::events::EventKind;
    use crate::bindings::types::{ErrorKind, HostError};

    #[derive(Default)]
    pub(crate) struct FakeHost {
        next_handle: u64,
        pub(crate) listeners: HashMap<u64, EventKind>,
        pub(crate) commands: HashMap<String, u64>,
        pub(crate) tasks: HashMap<u64, (u64, bool)>,
        pub(crate) cancelled: Vec<u64>,
        pub(crate) codec_filters: Vec<(String, u64)>,
        pub(crate) limbo_handlers: Vec<(String, u64)>,
        pub(crate) refused: HashSet<String>,
    }

    impl FakeHost {
        fn next_handle(&mut self) -> u64 {
            self.next_handle += 1;
            self.next_handle
        }

        fn schedule(&mut self, handler: u64, repeating: bool) -> u64 {
            let handle = self.next_handle();
            self.tasks.insert(handle, (handler, repeating));
            handle
        }

        fn refuse(&self, name: &str) -> Result<(), HostError> {
            if self.refused.contains(name) {
                Err(HostError {
                    kind: ErrorKind::Conflict,
                    message: format!("{name} is refused"),
                })
            } else {
                Ok(())
            }
        }
    }

    thread_local! {
        static FAKE: RefCell<FakeHost> = RefCell::new(FakeHost::default());
    }

    pub(crate) fn with_fake<R>(f: impl FnOnce(&mut FakeHost) -> R) -> R {
        FAKE.with(|host| f(&mut host.borrow_mut()))
    }

    pub(crate) fn subscribe(kind: EventKind, _priority: u8) -> Result<u64, HostError> {
        with_fake(|host| {
            host.refuse("subscribe")?;
            let handle = host.next_handle();
            host.listeners.insert(handle, kind);
            Ok(handle)
        })
    }

    pub(crate) fn unsubscribe(listener: u64) -> Result<bool, HostError> {
        Ok(with_fake(|host| host.listeners.remove(&listener).is_some()))
    }

    pub(crate) fn register_command(
        spec: &CommandSpec,
        handler: u64,
    ) -> Result<CommandRegistration, HostError> {
        with_fake(|host| {
            let name = spec.name.to_lowercase();
            host.refuse(&name)?;
            host.commands.insert(name.clone(), handler);
            Ok(CommandRegistration {
                namespaced: format!("fake:{name}"),
                name,
                aliases: spec.aliases.clone(),
                rejected_aliases: Vec::new(),
            })
        })
    }

    pub(crate) fn unregister_command(name: &str) -> Result<(), HostError> {
        with_fake(|host| {
            host.commands.remove(&name.to_lowercase());
        });
        Ok(())
    }

    pub(crate) fn delay(_after_ms: u64, handler: u64) -> Result<u64, HostError> {
        with_fake(|host| {
            host.refuse("scheduler")?;
            Ok(host.schedule(handler, false))
        })
    }

    pub(crate) fn interval(
        _period_ms: u64,
        _initial_delay_ms: Option<u64>,
        handler: u64,
    ) -> Result<u64, HostError> {
        with_fake(|host| {
            host.refuse("scheduler")?;
            Ok(host.schedule(handler, true))
        })
    }

    pub(crate) fn cancel(handle: u64) -> Result<(), HostError> {
        with_fake(|host| {
            host.tasks.remove(&handle);
            host.cancelled.push(handle);
        });
        Ok(())
    }

    pub(crate) fn register_codec_filter(
        metadata: &CodecFilterMetadata,
        factory: u64,
    ) -> Result<(), HostError> {
        with_fake(|host| {
            host.refuse(&metadata.id)?;
            host.codec_filters.push((metadata.id.clone(), factory));
            Ok(())
        })
    }

    pub(crate) fn register_limbo_handler(name: &str, handler: u64) -> Result<(), HostError> {
        with_fake(|host| {
            host.refuse(name)?;
            host.limbo_handlers.push((name.to_owned(), handler));
            Ok(())
        })
    }
}
