use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::{HandlerResult, LimboHandler, LimboSession, SessionEndReason};
use infrarust_api::types::{Component, PlayerId};
use wasmtime::component::Resource;

use crate::actor::InstanceRef;
use crate::convert;
use crate::plugin::call_guest;
use crate::registrations::{Binding, Registrations};

pub(crate) struct WasmLimboHandler {
    binding: Arc<Binding>,
    name: String,
    instance: InstanceRef,
    registrations: Arc<Registrations>,
}

impl WasmLimboHandler {
    pub(crate) fn new(
        binding: Arc<Binding>,
        name: String,
        instance: InstanceRef,
        registrations: Arc<Registrations>,
    ) -> Self {
        Self {
            binding,
            name,
            instance,
            registrations,
        }
    }
}

pub(crate) fn deny_unavailable() -> HandlerResult {
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
        let binding = Arc::clone(&self.binding);
        let name = self.name.clone();
        let handle = session.handle();
        let arc_session = handle.as_session();
        Box::pin(async move {
            call_guest(instance, "limbo-on-player-enter", move |store, bindings| {
                Box::pin(async move {
                    let generation = store.data().generation();
                    let Some(handler_id) = binding.callback_for(generation) else {
                        return Ok(deny_unavailable());
                    };
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
                    let result = outcome.map(convert::handler_result_from_wit)?;
                    if matches!(
                        result,
                        HandlerResult::Hold | HandlerResult::HoldWithTimeout { .. }
                    ) {
                        store.data().registrations().track_hold(handle, generation);
                    }
                    Ok(result)
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
        let binding = Arc::clone(&self.binding);
        let name = self.name.clone();
        let arc_session = session.handle().as_session();
        let command = command.to_string();
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-command", move |store, bindings| {
                Box::pin(async move {
                    let Some(handler_id) = binding.callback_for(store.data().generation()) else {
                        return Ok(());
                    };
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
        let binding = Arc::clone(&self.binding);
        let name = self.name.clone();
        let arc_session = session.handle().as_session();
        let message = message.to_string();
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-chat", move |store, bindings| {
                Box::pin(async move {
                    let Some(handler_id) = binding.callback_for(store.data().generation()) else {
                        return Ok(());
                    };
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
        self.registrations.release_hold(player_id);
        let instance = self.instance.clone();
        let binding = Arc::clone(&self.binding);
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-disconnect", move |store, bindings| {
                Box::pin(async move {
                    let Some(handler_id) = binding.callback_for(store.data().generation()) else {
                        return Ok(());
                    };
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
        self.registrations.release_hold(player_id);
        let instance = self.instance.clone();
        let binding = Arc::clone(&self.binding);
        let wit_reason = convert::session_end_reason_to_wit(reason);
        Box::pin(async move {
            let _ = call_guest(instance, "limbo-on-session-end", move |store, bindings| {
                Box::pin(async move {
                    let Some(handler_id) = binding.callback_for(store.data().generation()) else {
                        return Ok(());
                    };
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
