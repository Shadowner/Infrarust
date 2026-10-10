use std::time::Duration;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::{
    HandlerResult, LimboEntryContext, LimboHandler, LimboHandlerError, LimboSession, SessionHandle,
};
use tokio::sync::mpsc;

use crate::error::{HarnessError, HarnessResult};
use crate::scripted::ScriptedPlugin;

pub struct Hold {
    pub handle: SessionHandle,
    pub entry: LimboEntryContext,
}

pub type Holds = mpsc::UnboundedReceiver<Result<Hold, LimboHandlerError>>;

type Sender = mpsc::UnboundedSender<Result<Hold, LimboHandlerError>>;

struct HoldingHandler {
    name: String,
    held: Sender,
}

impl LimboHandler for HoldingHandler {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let hold = Hold {
            handle: session.handle(),
            entry: session.entry_context().clone(),
        };
        let _ = self.held.send(Ok(hold));
        Box::pin(async { HandlerResult::Hold })
    }
}

fn handler(name: &str, held: &Sender) -> Box<dyn LimboHandler> {
    Box::new(HoldingHandler {
        name: name.to_owned(),
        held: held.clone(),
    })
}

#[must_use]
pub fn holding_handler(handler_name: &str) -> (Box<dyn LimboHandler>, Holds) {
    let (held, holds) = mpsc::unbounded_channel();
    (handler(handler_name, &held), holds)
}

#[must_use]
pub fn holding_gate(plugin_id: &str, handler_name: &str) -> (ScriptedPlugin, Holds) {
    let (held, holds) = mpsc::unbounded_channel();
    let name = handler_name.to_owned();
    let plugin = ScriptedPlugin::new(plugin_id).on_enable(move |ctx| {
        if let Err(error) = ctx.register_limbo_handler(handler(&name, &held)) {
            let _ = held.send(Err(error));
        }
    });
    (plugin, holds)
}

pub async fn next_hold(holds: &mut Holds, timeout: Duration) -> HarnessResult<Hold> {
    match tokio::time::timeout(timeout, holds.recv()).await {
        Ok(Some(Ok(hold))) => Ok(hold),
        Ok(Some(Err(error))) => Err(HarnessError::setup(format!(
            "the limbo handler did not register: {error}"
        ))),
        Ok(None) => Err(HarnessError::Closed("the limbo handler".to_owned())),
        Err(_) => Err(HarnessError::timeout("a player held in limbo", timeout)),
    }
}
