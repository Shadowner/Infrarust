use infrarust_api::command::CommandContext;
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
            let Some(player_id) = ctx.player_id else {
                return;
            };
            let Some(sender) = services.player_registry.get_player_by_id(player_id) else {
                return;
            };

            let Some(target_name) = args.first() else {
                let _ = sender.send_message(ProxyMessage::error("Usage: /ir find <player>"));
                return;
            };

            match services.player_registry.get_player(target_name) {
                Some(target) => match target.current_server() {
                    Some(server) => {
                        let _ = sender.send_message(ProxyMessage::success(&format!(
                            "{target_name} is on server: {}",
                            server.as_str()
                        )));
                    }
                    None => {
                        let _ = sender.send_message(ProxyMessage::info(&format!(
                            "{target_name} is online but not on any server."
                        )));
                    }
                },
                None => {
                    let _ = sender.send_message(ProxyMessage::error(&format!(
                        "Player '{target_name}' is not online."
                    )));
                }
            }
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
