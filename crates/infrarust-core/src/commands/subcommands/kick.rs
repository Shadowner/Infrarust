use infrarust_api::command::{CommandContext, CommandSource};
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;

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

            let Some(target_name) = args.first() else {
                sender.send_message(ProxyMessage::error("Usage: /ir kick <player> [reason]"));
                return;
            };

            let Some(target) = services.player_registry.get_player(target_name) else {
                sender.send_message(ProxyMessage::error(&format!(
                    "Player '{target_name}' is not online."
                )));
                return;
            };

            let reason = if args.len() > 1 {
                args[1..].join(" ")
            } else {
                "Kicked by proxy".to_string()
            };

            target.disconnect(Component::text(&reason)).await;

            sender.send_message(ProxyMessage::success(&format!(
                "Kicked {target_name}: {reason}"
            )));
        })
    }

    fn tab_complete<'a>(
        &'a self,
        args: &'a [String],
        _source: &'a CommandSource,
        services: &'a CommandServices,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            if args.len() <= 1 {
                let prefix = args.first().map(String::as_str).unwrap_or("");
                services
                    .player_registry
                    .get_all_players()
                    .into_iter()
                    .map(|p| p.profile().username.clone())
                    .filter(|name| name.to_lowercase().starts_with(&prefix.to_lowercase()))
                    .collect()
            } else {
                vec![]
            }
        })
    }
}
