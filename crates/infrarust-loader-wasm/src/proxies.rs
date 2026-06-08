//! Marker + proxy bridge for guest-registered command and scheduler callbacks.

use std::sync::Weak;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::PlayerId;
use tokio::sync::Mutex;

use crate::plugin::WasmInstance;

pub(crate) struct WasmCommandHandler {
    callback_id: u64,
    instance: Weak<Mutex<WasmInstance>>,
    plugin_id: String,
}

impl WasmCommandHandler {
    pub(crate) fn new(
        callback_id: u64,
        instance: Weak<Mutex<WasmInstance>>,
        plugin_id: String,
    ) -> Self {
        Self {
            callback_id,
            instance,
            plugin_id,
        }
    }
}

impl CommandHandler for WasmCommandHandler {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        _player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        let instance = self.instance.clone();
        let plugin_id = self.plugin_id.clone();
        let callback_id = self.callback_id;
        Box::pin(async move {
            let Some(arc) = instance.upgrade() else {
                return;
            };
            let mut guard = arc.lock().await;
            let WasmInstance { store, bindings } = &mut *guard;
            if store.data().is_poisoned() {
                return;
            }
            store.data_mut().reset_epoch_budget();
            let player = ctx.player_id.map(PlayerId::as_u64);
            match bindings
                .infrarust_plugin_guest()
                .call_handle_command(&mut *store, callback_id, &ctx.args, player)
                .await
            {
                Ok(()) => {}
                Err(trap) => {
                    tracing::error!(plugin = %plugin_id, callback = callback_id, error = %trap,
                        "wasm guest trapped in handle-command; poisoning instance");
                    store.data_mut().set_poisoned();
                }
            }
        })
    }

    fn tab_complete<'a>(
        &'a self,
        partial_args: Vec<String>,
        cursor: u32,
    ) -> BoxFuture<'a, Vec<String>> {
        let instance = self.instance.clone();
        let plugin_id = self.plugin_id.clone();
        let callback_id = self.callback_id;
        Box::pin(async move {
            let Some(arc) = instance.upgrade() else {
                return Vec::new();
            };
            let mut guard = arc.lock().await;
            let WasmInstance { store, bindings } = &mut *guard;
            if store.data().is_poisoned() {
                return Vec::new();
            }
            store.data_mut().reset_epoch_budget();
            match bindings
                .infrarust_plugin_guest()
                .call_tab_complete(&mut *store, callback_id, &partial_args, cursor)
                .await
            {
                Ok(suggestions) => suggestions,
                Err(trap) => {
                    tracing::error!(plugin = %plugin_id, callback = callback_id, error = %trap,
                        "wasm guest trapped in tab-complete; poisoning instance");
                    store.data_mut().set_poisoned();
                    Vec::new()
                }
            }
        })
    }
}

pub(crate) fn dispatch_scheduled_task(
    instance: Weak<Mutex<WasmInstance>>,
    callback_id: u64,
    plugin_id: String,
) {
    tokio::spawn(async move {
        let Some(arc) = instance.upgrade() else {
            return;
        };
        let mut guard = arc.lock().await;
        let WasmInstance { store, bindings } = &mut *guard;
        if store.data().is_poisoned() {
            return;
        }
        store.data_mut().reset_epoch_budget();
        match bindings
            .infrarust_plugin_guest()
            .call_on_scheduled_task(&mut *store, callback_id)
            .await
        {
            Ok(()) => {}
            Err(trap) => {
                tracing::error!(plugin = %plugin_id, callback = callback_id, error = %trap,
                    "wasm guest trapped in on-scheduled-task; poisoning instance");
                store.data_mut().set_poisoned();
            }
        }
    });
}
