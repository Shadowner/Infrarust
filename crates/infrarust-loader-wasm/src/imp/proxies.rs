use std::sync::Arc;

use infrarust_api::command::{
    CommandContext, CommandHandler, CommandSource, SuggestContext, Suggestion,
};
use infrarust_api::event::BoxFuture;

use crate::actor::InstanceRef;
use crate::bindings::exports::infrarust::plugin::guest as wg;
use crate::component;
use crate::convert;
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

pub(crate) fn command_sender(source: &CommandSource) -> wg::CommandSender {
    match source.player() {
        Some(player) => wg::CommandSender::Player(convert::player_ref(&**player)),
        None => wg::CommandSender::Console,
    }
}

fn suggestion_from_wit(suggestion: wg::Suggestion, instance: &InstanceRef) -> Suggestion {
    let text = Suggestion::new(suggestion.text);
    let Some(tooltip) = suggestion.tooltip else {
        return text;
    };
    match component::from_wit(&tooltip) {
        Ok(tooltip) => text.with_tooltip(tooltip),
        Err(error) => {
            if let Some(suppressed) = instance.admit_warning() {
                tracing::warn!(plugin = instance.plugin_id(), %error, suppressed,
                    "wasm plugin suggested a tooltip with an invalid text component; dropping the tooltip");
            }
            text
        }
    }
}

impl CommandHandler for WasmCommandHandler {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        let instance = self.instance.clone();
        let binding = Arc::clone(&self.binding);
        Box::pin(async move {
            let invocation = wg::CommandInvocation {
                sender: command_sender(&ctx.source),
                label: ctx.label,
                args: ctx.args,
                raw: ctx.raw,
            };
            let _ = call_guest(instance, "handle-command", move |store, bindings| {
                Box::pin(async move {
                    let Some(handler) = binding.callback_for(store.data().generation()) else {
                        return Ok(());
                    };
                    bindings
                        .infrarust_plugin_guest()
                        .call_handle_command(&mut *store, handler, &invocation)
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
        let sender = command_sender(&ctx.source);
        Box::pin(async move {
            let caller = instance.clone();
            call_guest(caller, "tab-complete", move |store, bindings| {
                Box::pin(async move {
                    let Some(handler) = binding.callback_for(store.data().generation()) else {
                        return Ok(Vec::new());
                    };
                    bindings
                        .infrarust_plugin_guest()
                        .call_tab_complete(&mut *store, handler, &sender, &ctx.args, cursor)
                        .await
                })
            })
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|suggestion| suggestion_from_wit(suggestion, &instance))
            .collect()
        })
    }
}

pub(crate) fn dispatch_scheduled_task(instance: InstanceRef, handler: u64) {
    tokio::spawn(async move {
        let _ = call_guest(instance, "on-scheduled-task", move |store, bindings| {
            Box::pin(async move {
                bindings
                    .infrarust_plugin_guest()
                    .call_on_scheduled_task(&mut *store, handler)
                    .await
            })
        })
        .await;
    });
}
