use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;
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

    fn required_level(&self) -> PermissionLevel {
        PermissionLevel::Admin
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
            let Some(player_id) = ctx.player_id else {
                return;
            };
            let Some(sender) = services.player_registry.get_player_by_id(player_id) else {
                return;
            };

            let Some(target_name) = args.first() else {
                let _ =
                    sender.send_message(ProxyMessage::error("Usage: /ir kick <player> [reason]"));
                return;
            };

            let Some(target) = services.player_registry.get_player(target_name) else {
                let _ = sender.send_message(ProxyMessage::error(&format!(
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

            let _ = sender.send_message(ProxyMessage::success(&format!(
                "Kicked {target_name}: {reason}"
            )));
        })
    }

    fn tab_complete(&self, args: &[&str], services: &CommandServices) -> Vec<String> {
        if args.len() <= 1 {
            let prefix = args.first().copied().unwrap_or("");
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
    }
}
