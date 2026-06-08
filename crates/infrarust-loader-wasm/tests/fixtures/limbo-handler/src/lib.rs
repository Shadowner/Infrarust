use std::cell::RefCell;
use std::collections::HashSet;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct LimboPlugin;

struct Gate {
    waiting: RefCell<HashSet<u64>>,
}

impl LimboHandler for Gate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        self.waiting.borrow_mut().insert(session.player_id());
        session
            .send_message(Component::text("Type /continue to proceed"))
            .ok();
        HandlerOutcome::Hold
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        match command {
            "continue" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session.complete(HandlerOutcome::Accept);
            }
            "redirect" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session.complete(HandlerOutcome::Redirect("hub".to_string()));
            }
            _ => {
                session.send_message(Component::text("Unknown command")).ok();
            }
        }
    }

    fn on_chat(&self, session: &LimboSession, _message: &str) {
        session
            .send_message(Component::text("Please use /continue"))
            .ok();
    }

    fn on_disconnect(&self, player_id: u64) {
        self.waiting.borrow_mut().remove(&player_id);
    }
}

struct Boom;

impl LimboHandler for Boom {
    fn on_player_enter(&self, _session: &LimboSession) -> HandlerOutcome {
        panic!("boom: this handler always traps");
    }
}

#[plugin(id = "limbo-handler", name = "Limbo Handler Fixture")]
impl Plugin for LimboPlugin {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
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
    }
}
