use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::{CommandContext, CommandSource};
use infrarust_api::event::BoxFuture;

use crate::commands::actions::kick_player;
use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct KickSubcommand;

impl SubcommandHandler for KickSubcommand {
    fn name(&self) -> &str {
        "kick"
    }

    fn description(&self) -> &str {
        "Kick a player from the proxy"
    }

    fn admin_only(&self) -> bool {
        true
    }

    fn usage(&self) -> &str {
        "/ir kick <player> [reason]"
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let sender = &ctx.source;

            let Some((target_name, reason)) = args.split_first() else {
                sender.send_message(ProxyMessage::error("Usage: /ir kick <player> [reason]"));
                return;
            };
            let reason = (!reason.is_empty()).then(|| reason.join(" "));

            let message = match kick_player(&*services.player_registry, target_name, reason).await {
                Ok(kicked) => {
                    ProxyMessage::success(&format!("Kicked {}: {}", kicked.player, kicked.reason))
                }
                Err(error) => ProxyMessage::error(&error.to_string()),
            };
            sender.send_message(message);
        })
    }

    fn tab_complete<'a>(
        &'a self,
        args: &'a [String],
        _source: &'a CommandSource,
        services: &'a CommandServices,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            match args {
                [] => services.complete_player_names(""),
                [prefix] => services.complete_player_names(prefix),
                _ => Vec::new(),
            }
        })
    }
}
