use std::future::Future;
use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::{HandlerResult, LimboHandler, LimboSession, SessionEndReason};
use infrarust_api::types::PlayerId;
use wasmtime::Store;
use wasmtime::component::Resource;

use crate::actor::InstanceRef;
use crate::bindings::Plugin as PluginBindings;
use crate::bindings::infrarust::plugin::limbo as wl;
use crate::component;
use crate::convert;
use crate::registrations::{Binding, Registrations};
use crate::store_state::PluginStoreState;

type LentSession = Resource<wl::LimboSession>;

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

    fn call_handler<T, F>(
        &self,
        op: &'static str,
        call: F,
    ) -> impl Future<Output = Option<T>> + Send + 'static
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
                u64,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        let instance = self.instance.clone();
        let binding = Arc::clone(&self.binding);
        async move {
            instance
                .call_or_none(op, move |store, bindings| {
                    Box::pin(async move {
                        let Some(handler) = binding.callback_for(store.data().generation()) else {
                            return Ok(None);
                        };
                        call(store, bindings, handler).await.map(Some)
                    })
                })
                .await
                .flatten()
        }
    }

    fn lend_session<T, F>(
        &self,
        op: &'static str,
        refusal: &'static str,
        session: Arc<dyn LimboSession>,
        call: F,
    ) -> impl Future<Output = Option<T>> + Send + 'static
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
                u64,
                LentSession,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        let name = self.name.clone();
        let called = self.call_handler(op, move |store, bindings, handler| {
            Box::pin(async move {
                let lent = match store.data_mut().push_limbo_session(session) {
                    Ok(lent) => lent,
                    Err(e) => {
                        tracing::error!(plugin = %store.data().plugin_id(), handler = %name, error = %e,
                            "{refusal}");
                        return Ok(None);
                    }
                };
                let rep = lent.rep();
                let outcome = call(store, bindings, handler, lent).await;
                let _ = store.data_mut().drop_limbo_session(Resource::new_own(rep));
                outcome.map(Some)
            })
        });
        async move { called.await.flatten() }
    }
}

pub(crate) fn deny_unavailable() -> HandlerResult {
    HandlerResult::unavailable()
}

impl LimboHandler for WasmLimboHandler {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let handle = session.handle();
        let entered = self.lend_session(
            "limbo-on-player-enter",
            "failed to lend limbo session to guest; denying",
            handle.as_session(),
            move |store, bindings, handler, lent| {
                Box::pin(async move {
                    let generation = store.data().generation();
                    let outcome = bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_player_enter(&mut *store, handler, lent)
                        .await?;
                    let plugin = store.data().plugin_id().to_owned();
                    let result = convert::handler_result_with(&outcome, &mut |text| {
                        Ok(component::from_wit_or_fallback(
                            text,
                            &plugin,
                            "limbo handler result",
                        ))
                    })
                    .unwrap_or_else(|_| deny_unavailable());
                    if matches!(
                        result,
                        HandlerResult::Hold | HandlerResult::HoldWithTimeout { .. }
                    ) {
                        store.data().registrations().track_hold(handle, generation);
                    }
                    Ok(result)
                })
            },
        );
        Box::pin(async move { entered.await.unwrap_or_else(deny_unavailable) })
    }

    fn on_command<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        let command = command.to_string();
        let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let commanded = self.lend_session(
            "limbo-on-command",
            "failed to lend limbo session to guest; dropping command",
            session.handle().as_session(),
            move |store, bindings, handler, lent| {
                Box::pin(async move {
                    bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_command(&mut *store, handler, lent, &command, &args)
                        .await
                })
            },
        );
        Box::pin(async move {
            commanded.await;
        })
    }

    fn on_chat<'a>(&'a self, session: &'a dyn LimboSession, message: &'a str) -> BoxFuture<'a, ()> {
        let message = message.to_string();
        let chatted = self.lend_session(
            "limbo-on-chat",
            "failed to lend limbo session to guest; dropping chat",
            session.handle().as_session(),
            move |store, bindings, handler, lent| {
                Box::pin(async move {
                    bindings
                        .infrarust_plugin_guest()
                        .call_limbo_on_chat(&mut *store, handler, lent, &message)
                        .await
                })
            },
        );
        Box::pin(async move {
            chatted.await;
        })
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        self.registrations.release_hold(player_id);
        let told = self.call_handler("limbo-on-disconnect", move |store, bindings, handler| {
            Box::pin(async move {
                bindings
                    .infrarust_plugin_guest()
                    .call_limbo_on_disconnect(&mut *store, handler, player_id.as_u64())
                    .await
            })
        });
        Box::pin(async move {
            told.await;
        })
    }

    fn on_session_end(&self, player_id: PlayerId, reason: SessionEndReason) -> BoxFuture<'_, ()> {
        self.registrations.release_hold(player_id);
        let wit_reason = convert::session_end_reason_to_wit(reason);
        let told = self.call_handler("limbo-on-session-end", move |store, bindings, handler| {
            Box::pin(async move {
                bindings
                    .infrarust_plugin_guest()
                    .call_limbo_on_session_end(&mut *store, handler, player_id.as_u64(), wit_reason)
                    .await
            })
        });
        Box::pin(async move {
            told.await;
        })
    }
}
