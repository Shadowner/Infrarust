use infrarust_api::command::{CommandContext, CommandSource};
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::services::player_registry::PlayerRegistry;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct FindSubcommand;

impl SubcommandHandler for FindSubcommand {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find which server a player is on"
    }

    fn usage(&self) -> &str {
        "/ir find <player>"
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
                sender.send_message(ProxyMessage::error("Usage: /ir find <player>"));
                return;
            };

            match services.player_registry.get_player(target_name) {
                Some(target) => match target.current_server() {
                    Some(server) => {
                        sender.send_message(ProxyMessage::success(&format!(
                            "{target_name} is on server: {}",
                            server.as_str()
                        )));
                    }
                    None => {
                        sender.send_message(ProxyMessage::info(&format!(
                            "{target_name} is online but not on any server."
                        )));
                    }
                },
                None => {
                    sender.send_message(ProxyMessage::error(&format!(
                        "Player '{target_name}' is not online."
                    )));
                }
            }
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
