use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct LimboPlugin;

struct Gate {
    waiting: RefCell<HashSet<PlayerId>>,
}

impl LimboHandler for Gate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        self.waiting.borrow_mut().insert(session.player_id());
        session.send_message("Type /continue to proceed").ok();
        HandlerOutcome::Hold
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        match command {
            "continue" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session.complete(HandlerOutcome::Accept).ok();
            }
            "redirect" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session
                    .complete(HandlerOutcome::Redirect("hub".into()))
                    .ok();
            }
            _ => {
                session.send_message("Unknown command").ok();
            }
        }
    }

    fn on_chat(&self, session: &LimboSession, _message: &str) {
        session.send_message("Please use /continue").ok();
    }

    fn on_disconnect(&self, player: PlayerId) {
        self.waiting.borrow_mut().remove(&player);
    }
}

struct Boom;

impl LimboHandler for Boom {
    fn on_player_enter(&self, _session: &LimboSession) -> HandlerOutcome {
        panic!("boom: this handler always traps");
    }
}

struct TimedGate;

impl LimboHandler for TimedGate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        session.send_message("Type /continue within 5s").ok();
        HandlerOutcome::HoldWithTimeout {
            after: Duration::from_secs(5),
            on_timeout: TimeoutOutcome::Deny(Component::text("Timed out")),
        }
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        if command == "continue" {
            session.complete(HandlerOutcome::Accept).ok();
        }
    }

    fn on_session_end(&self, _player: PlayerId, _reason: SessionEndReason) {}
}

struct DelayedGate;

impl LimboHandler for DelayedGate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        let handle = session.handle();
        let scheduled = Context::new().delay(Duration::from_millis(50), move || {
            if !handle.cancelled() {
                handle.complete(HandlerOutcome::Accept).ok();
            }
        });
        match scheduled {
            Ok(_) => HandlerOutcome::Hold,
            Err(_) => HandlerOutcome::Accept,
        }
    }
}

#[plugin(id = "limbo-handler", name = "Limbo Handler Fixture")]
impl Plugin for LimboPlugin {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        Ok(())
    }

    fn register_limbo_handlers(reg: &mut LimboRegistrar) {
        reg.add(
            "gate",
            Gate {
                waiting: RefCell::new(HashSet::new()),
            },
        );
        reg.add("boom", Boom);
        reg.add("timed-gate", TimedGate);
        reg.add("delayed-gate", DelayedGate);
    }
}
