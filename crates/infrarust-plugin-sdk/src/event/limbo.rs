use super::GuestEvent;
use crate::bindings::events::{self as we, Event, EventKind};
use crate::component::{Component, from_host};
use crate::limbo::EntryContext;
use crate::types::{PlayerRef, ServerId};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LimboEnterEvent {
    pub player: PlayerRef,
    pub handlers: Vec<String>,
    pub context: EntryContext,
}

impl GuestEvent for LimboEnterEvent {
    const KIND: EventKind = EventKind::LimboEnter;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::LimboEnter(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            handlers: e.handlers,
            context: EntryContext::from_wit(e.context),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LimboExitReason {
    Released,
    Redirected,
    SentToLimbo(Vec<String>),
    Kicked(Component),
    Disconnected,
    TimedOut,
    Shutdown,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LimboExitEvent {
    pub player: PlayerRef,
    pub reason: LimboExitReason,
    pub next_server: Option<ServerId>,
}

impl GuestEvent for LimboExitEvent {
    const KIND: EventKind = EventKind::LimboExit;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::LimboExit(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            reason: match e.reason {
                we::LimboExitReason::Released => LimboExitReason::Released,
                we::LimboExitReason::Redirected => LimboExitReason::Redirected,
                we::LimboExitReason::SentToLimbo(handlers) => {
                    LimboExitReason::SentToLimbo(handlers)
                }
                we::LimboExitReason::Kicked(reason) => LimboExitReason::Kicked(from_host(reason)),
                we::LimboExitReason::Disconnected => LimboExitReason::Disconnected,
                we::LimboExitReason::TimedOut => LimboExitReason::TimedOut,
                we::LimboExitReason::Shutdown => LimboExitReason::Shutdown,
            },
            next_server: e.next_server.map(ServerId::from),
        })
    }
}
