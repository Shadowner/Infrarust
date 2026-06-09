use std::sync::Weak;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::{HandlerResult, LimboHandler, LimboSession, SessionEndReason};
use infrarust_api::types::{Component, PlayerId};
use tokio::sync::Mutex;
use wasmtime::component::Resource;

use crate::convert;
use crate::plugin::{WasmInstance, call_guest};

pub(crate) struct WasmLimboHandler {
    handler_id: u64,
    name: String,
    instance: Weak<Mutex<WasmInstance>>,
}

impl WasmLimboHandler {
    pub(crate) fn new(handler_id: u64, name: String, instance: Weak<Mutex<WasmInstance>>) -> Self {
        Self {
            handler_id,
            name,
            instance,
        }
    }
}

fn deny_unavailable() -> HandlerResult {
    HandlerResult::Deny(Component::text("Limbo handler unavailable"))
}

impl LimboHandler for WasmLimboHandler {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let instance = self.instance.clone();
        let name = self.name.clone();
        let handler_id = self.handler_id;
        let arc_session = session.handle().as_session();
        Box::pin(async move {
            call_guest(instance, "limbo-on-player-enter", move |store, bindings| {
                Box::pin(async move {
                    let res = match store.data_mut().push_limbo_session(arc_session) {
                        Ok(res) => res,
                        Err(e) => {
                            tracing::error!(plugin = %store.data().plugin_id, handler = %name, error = %e,
                                "failed to lend limbo session to guest; denying");
                            return Ok(deny_unavailable());
                        }
                    };
                    let rep = res.rep();
                    let outcome = bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_player_enter(&mut *store, handler_id, res)
                        .await;
                    let _ = store.data_mut().drop_limbo_session(Resource::new_own(rep));
                    outcome.map(convert::handler_result_from_wit)
                })
            })
            .await
            .unwrap_or_else(deny_unavailable)
        })
    }

    fn on_command<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        let instance = self.instance.clone();
        let name = self.name.clone();
        let handler_id = self.handler_id;
        let arc_session = session.handle().as_session();
        let command = command.to_string();
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-command", move |store, bindings| {
                Box::pin(async move {
                    let res = match store.data_mut().push_limbo_session(arc_session) {
                        Ok(res) => res,
                        Err(e) => {
                            tracing::error!(plugin = %store.data().plugin_id, handler = %name, error = %e,
                                "failed to lend limbo session to guest; dropping command");
                            return Ok(());
                        }
                    };
                    let rep = res.rep();
                    let outcome = bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_command(&mut *store, handler_id, res, &command, &args)
                        .await;
                    let _ = store.data_mut().drop_limbo_session(Resource::new_own(rep));
                    outcome
                })
            })
            .await;
        })
    }

    fn on_chat<'a>(&'a self, session: &'a dyn LimboSession, message: &'a str) -> BoxFuture<'a, ()> {
        let instance = self.instance.clone();
        let name = self.name.clone();
        let handler_id = self.handler_id;
        let arc_session = session.handle().as_session();
        let message = message.to_string();
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-chat", move |store, bindings| {
                Box::pin(async move {
                    let res = match store.data_mut().push_limbo_session(arc_session) {
                        Ok(res) => res,
                        Err(e) => {
                            tracing::error!(plugin = %store.data().plugin_id, handler = %name, error = %e,
                                "failed to lend limbo session to guest; dropping chat");
                            return Ok(());
                        }
                    };
                    let rep = res.rep();
                    let outcome = bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_chat(&mut *store, handler_id, res, &message)
                        .await;
                    let _ = store.data_mut().drop_limbo_session(Resource::new_own(rep));
                    outcome
                })
            })
            .await;
        })
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        let instance = self.instance.clone();
        let handler_id = self.handler_id;
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-disconnect", move |store, bindings| {
                Box::pin(async move {
                    bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_disconnect(&mut *store, handler_id, player_id.as_u64())
                        .await
                })
            })
            .await;
        })
    }

    fn on_session_end(&self, player_id: PlayerId, reason: SessionEndReason) -> BoxFuture<'_, ()> {
        let instance = self.instance.clone();
        let handler_id = self.handler_id;
        let wit_reason = convert::session_end_reason_to_wit(reason);
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-session-end", move |store, bindings| {
                Box::pin(async move {
                    bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_session_end(
                            &mut *store,
                            handler_id,
                            player_id.as_u64(),
                            wit_reason,
                        )
                        .await
                })
            })
            .await;
        })
    }
}
