//! Marker + proxy bridge for guest-registered command and scheduler callbacks.

use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler, SuggestContext, Suggestion};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::PlayerId;

use crate::actor::InstanceRef;
use crate::plugin::call_guest;
use crate::registrations::Binding;

pub(crate) struct WasmCommandHandler {
    binding: Arc<Binding>,
    instance: InstanceRef,
}

impl WasmCommandHandler {
    pub(crate) fn new(binding: Arc<Binding>, instance: InstanceRef) -> Self {
        Self { binding, instance }
    }
}

impl CommandHandler for WasmCommandHandler {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        let instance = self.instance.clone();
        let binding = Arc::clone(&self.binding);
        Box::pin(async move {
            let player = ctx.source.player_id().map(PlayerId::as_u64);
            let _ = call_guest(instance, "handle-command", move |store, bindings| {
                Box::pin(async move {
                    let Some(callback_id) = binding.callback_for(store.data().generation()) else {
                        return Ok(());
                    };
                    bindings
                        .infrarust_plugin_guest()
                        .call_handle_command(&mut *store, callback_id, &ctx.args, player)
                        .await
                })
            })
            .await;
        })
    }

    fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        let instance = self.instance.clone();
        let binding = Arc::clone(&self.binding);
        let cursor = u32::try_from(ctx.raw_args.len()).unwrap_or(u32::MAX);
        Box::pin(async move {
            call_guest(instance, "tab-complete", move |store, bindings| {
                Box::pin(async move {
                    let Some(callback_id) = binding.callback_for(store.data().generation()) else {
                        return Ok(Vec::new());
                    };
                    bindings
                        .infrarust_plugin_guest()
                        .call_tab_complete(&mut *store, callback_id, &ctx.args, cursor)
                        .await
                })
            })
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Suggestion::new)
            .collect()
        })
    }
}

pub(crate) fn dispatch_scheduled_task(instance: InstanceRef, callback_id: u64) {
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
