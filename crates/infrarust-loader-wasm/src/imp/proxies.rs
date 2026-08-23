//! Marker + proxy bridge for guest-registered command and scheduler callbacks.

use std::sync::Weak;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::PlayerId;
use tokio::sync::Mutex;

use crate::plugin::{WasmInstance, call_guest};

pub(crate) struct WasmCommandHandler {
    callback_id: u64,
    instance: Weak<Mutex<WasmInstance>>,
}

impl WasmCommandHandler {
    pub(crate) fn new(callback_id: u64, instance: Weak<Mutex<WasmInstance>>) -> Self {
        Self {
            callback_id,
            instance,
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
        let callback_id = self.callback_id;
        Box::pin(async move {
            let player = ctx.player_id.map(PlayerId::as_u64);
            let _ = call_guest(instance, "handle-command", move |store, bindings| {
                Box::pin(async move {
                    bindings
                        .infrarust_plugin_guest()
                        .call_handle_command(&mut *store, callback_id, &ctx.args, player)
                        .await
                })
            })
            .await;
        })
    }

    fn tab_complete<'a>(
        &'a self,
        partial_args: Vec<String>,
        cursor: u32,
    ) -> BoxFuture<'a, Vec<String>> {
        let instance = self.instance.clone();
        let callback_id = self.callback_id;
        Box::pin(async move {
            call_guest(instance, "tab-complete", move |store, bindings| {
                Box::pin(async move {
                    bindings
                        .infrarust_plugin_guest()
                        .call_tab_complete(&mut *store, callback_id, &partial_args, cursor)
                        .await
                })
            })
            .await
            .unwrap_or_default()
        })
    }
}

pub(crate) fn dispatch_scheduled_task(instance: Weak<Mutex<WasmInstance>>, callback_id: u64) {
    tokio::spawn(async move {
        let _ = call_guest(instance, "on-scheduled-task", move |store, bindings| {
            Box::pin(async move {
                bindings
                    .infrarust_plugin_guest()
                    .call_on_scheduled_task(&mut *store, callback_id)
                    .await
            })
        })
        .await;
    });
}
