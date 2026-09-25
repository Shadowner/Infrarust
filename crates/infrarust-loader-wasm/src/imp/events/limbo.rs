use infrarust_api::events::limbo::{LimboEnterEvent, LimboExitEvent, LimboExitReason};

use super::WasmEvent;
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::component;
use crate::convert;

impl WasmEvent for LimboEnterEvent {
    const KIND: EventKind = EventKind::LimboEnter;

    fn to_wit(&self) -> we::Event {
        we::Event::LimboEnter(we::LimboEnterEvent {
            player: convert::player_ref(&*self.player),
            handlers: self.handlers.clone(),
            context: convert::limbo_entry_context_to_wit(&self.context),
        })
    }
}

fn exit_reason(reason: &LimboExitReason) -> we::LimboExitReason {
    match reason {
        LimboExitReason::Released => we::LimboExitReason::Released,
        LimboExitReason::Redirected => we::LimboExitReason::Redirected,
        LimboExitReason::SentToLimbo { handlers } => {
            we::LimboExitReason::SentToLimbo(handlers.clone())
        }
        LimboExitReason::Kicked { reason } => {
            we::LimboExitReason::Kicked(component::to_wit(reason))
        }
        LimboExitReason::TimedOut => we::LimboExitReason::TimedOut,
        LimboExitReason::Shutdown => we::LimboExitReason::Shutdown,
        _ => we::LimboExitReason::Disconnected,
    }
}

impl WasmEvent for LimboExitEvent {
    const KIND: EventKind = EventKind::LimboExit;

    fn to_wit(&self) -> we::Event {
        we::Event::LimboExit(we::LimboExitEvent {
            player: convert::player_ref(&*self.player),
            reason: exit_reason(&self.reason),
            next_server: self.next_server.as_ref().map(|s| s.as_str().to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::limbo::context::LimboEntryContext;
    use infrarust_api::types::{Component, ServerId};

    use super::super::steve;
    use super::*;
    use crate::bindings::infrarust::plugin::limbo as wl;

    #[test]
    fn a_limbo_entry_carries_its_handlers_and_why() {
        let event = LimboEnterEvent::new(
            steve(),
            vec!["gate".into(), "queue".into()],
            LimboEntryContext::KickedFromServer {
                server: ServerId::new("survival"),
                reason: Component::text("restarting"),
            },
        );
        let we::Event::LimboEnter(record) = event.to_wit() else {
            panic!("a limbo entry is sent as limbo-enter");
        };
        assert_eq!(record.handlers, ["gate", "queue"]);
        assert_eq!(
            record.context,
            wl::LimboEntryContext::KickedFromServer((
                "survival".into(),
                component::to_wit(&Component::text("restarting"))
            ))
        );
    }

    #[test]
    fn a_limbo_exit_carries_the_next_server() {
        let event = LimboExitEvent::new(
            steve(),
            LimboExitReason::SentToLimbo {
                handlers: vec!["queue".into()],
            },
            Some(ServerId::new("lobby")),
        );
        let we::Event::LimboExit(record) = event.to_wit() else {
            panic!("a limbo exit is sent as limbo-exit");
        };
        assert_eq!(
            record.reason,
            we::LimboExitReason::SentToLimbo(vec!["queue".into()])
        );
        assert_eq!(record.next_server.as_deref(), Some("lobby"));
    }
}
