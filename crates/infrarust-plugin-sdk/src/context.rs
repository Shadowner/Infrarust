use std::rc::Rc;
use std::time::Duration;

use crate::ban_provider::BanProvider;
use crate::bindings::guest as wg;
use crate::command::CommandBuilder;
use crate::error::Error;
use crate::event::{
    EventPriority, GuestEvent, NamedEvent, NamedOutcome, PacketFilter, RawPacketEvent,
};
use crate::permissions::PermissionProvider;
use crate::runtime;
use crate::types::millis;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecoveryInfo {
    pub attempt: u32,
    pub cause: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnableReason {
    Initial,
    Recovered(RecoveryInfo),
}

impl EnableReason {
    pub(crate) fn from_wit(reason: wg::EnableReason) -> Self {
        match reason {
            wg::EnableReason::Initial => Self::Initial,
            wg::EnableReason::Recovered(info) => Self::Recovered(RecoveryInfo {
                attempt: info.attempt,
                cause: info.cause,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DisableReason {
    Shutdown,
    Unload,
    Quarantine,
}

impl DisableReason {
    pub(crate) const fn from_wit(reason: wg::DisableReason) -> Self {
        match reason {
            wg::DisableReason::Shutdown => Self::Shutdown,
            wg::DisableReason::Unload => Self::Unload,
            wg::DisableReason::Quarantine => Self::Quarantine,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskHandle(u64);

impl TaskHandle {
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn id(self) -> u64 {
        self.0
    }

    pub fn cancel(self) {
        runtime::cancel_task(self.0);
    }
}

#[derive(Debug)]
pub struct EventSubscription {
    id: u64,
}

impl EventSubscription {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    pub fn cancel(self) {
        runtime::unsubscribe_event(self.id);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Ambient,
    Enabling(EnableReason),
    Disabling(DisableReason),
}

#[derive(Debug, Clone, Default)]
pub struct Context {
    phase: Phase,
}

impl Context {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn enabling(reason: EnableReason) -> Self {
        Self {
            phase: Phase::Enabling(reason),
        }
    }

    pub(crate) fn disabling(reason: DisableReason) -> Self {
        Self {
            phase: Phase::Disabling(reason),
        }
    }

    #[must_use]
    pub const fn enable_reason(&self) -> Option<&EnableReason> {
        match &self.phase {
            Phase::Enabling(reason) => Some(reason),
            Phase::Ambient | Phase::Disabling(_) => None,
        }
    }

    #[must_use]
    pub const fn disable_reason(&self) -> Option<DisableReason> {
        match &self.phase {
            Phase::Disabling(reason) => Some(*reason),
            Phase::Ambient | Phase::Enabling(_) => None,
        }
    }

    pub fn on<E: GuestEvent>(
        &self,
        priority: EventPriority,
        handler: impl FnMut(&mut E) + 'static,
    ) -> Result<EventSubscription, Error> {
        runtime::register_event::<E>(priority, handler).map(|id| EventSubscription { id })
    }

    pub fn on_named(
        &self,
        name: impl Into<String>,
        priority: EventPriority,
        handler: impl FnMut(&mut NamedEvent) + 'static,
    ) -> Result<EventSubscription, Error> {
        let name = name.into();
        runtime::register_listener::<NamedEvent>(
            |priority| crate::host::subscribe_named(&name, priority),
            priority,
            handler,
        )
        .map(|id| EventSubscription { id })
    }

    pub fn fire_named(
        &self,
        name: &str,
        content_type: &str,
        payload: &[u8],
    ) -> Result<NamedOutcome, Error> {
        Ok(NamedOutcome::from_wit(crate::host::fire_named(
            name,
            content_type,
            payload,
        )?))
    }

    pub fn fire_named_text(&self, name: &str, text: &str) -> Result<NamedOutcome, Error> {
        self.fire_named(name, "text/plain", text.as_bytes())
    }

    pub fn on_packets(
        &self,
        filters: &[PacketFilter],
        priority: EventPriority,
        handler: impl FnMut(&mut RawPacketEvent) + 'static,
    ) -> Result<EventSubscription, Error> {
        let filters: Vec<_> = filters.iter().map(|filter| filter.to_wit()).collect();
        runtime::register_listener::<RawPacketEvent>(
            |priority| crate::host::subscribe_packets(&filters, priority),
            priority,
            handler,
        )
        .map(|id| EventSubscription { id })
    }

    pub fn command(&self, name: impl Into<String>) -> CommandBuilder {
        CommandBuilder::new(name.into())
    }

    pub fn unregister_command(&self, name: &str) -> Result<bool, Error> {
        runtime::unregister_command(name)
    }

    pub fn unregister_codec_filter(&self, id: &str) -> Result<(), Error> {
        runtime::unregister_codec_filter(id)
    }

    pub fn delay(
        &self,
        after: Duration,
        task: impl FnOnce() + 'static,
    ) -> Result<TaskHandle, Error> {
        runtime::schedule_delay(millis(after), Box::new(task)).map(TaskHandle::new)
    }

    pub fn interval(
        &self,
        period: Duration,
        task: impl FnMut() + 'static,
    ) -> Result<TaskHandle, Error> {
        runtime::schedule_interval(millis(period), None, Box::new(task)).map(TaskHandle::new)
    }

    pub fn interval_with_delay(
        &self,
        period: Duration,
        initial_delay: Duration,
        task: impl FnMut() + 'static,
    ) -> Result<TaskHandle, Error> {
        runtime::schedule_interval(millis(period), Some(millis(initial_delay)), Box::new(task))
            .map(TaskHandle::new)
    }

    pub fn cancel(&self, handle: TaskHandle) {
        handle.cancel();
    }

    pub fn provide_bans(&self, provider: impl BanProvider + 'static) -> Result<(), Error> {
        runtime::provide_bans(Rc::new(provider))
    }

    pub fn provide_permissions(
        &self,
        provider: impl PermissionProvider + 'static,
    ) -> Result<(), Error> {
        runtime::provide_permissions(Rc::new(provider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_knows_why_the_plugin_is_enabled_or_disabled() {
        let recovered = EnableReason::from_wit(wg::EnableReason::Recovered(wg::RecoveryInfo {
            attempt: 2,
            cause: "the guest trapped".into(),
        }));
        let ctx = Context::enabling(recovered.clone());
        assert_eq!(ctx.enable_reason(), Some(&recovered));
        assert_eq!(ctx.disable_reason(), None);

        let ctx = Context::disabling(DisableReason::from_wit(wg::DisableReason::Unload));
        assert_eq!(ctx.disable_reason(), Some(DisableReason::Unload));
        assert_eq!(Context::new().enable_reason(), None);
    }
}
