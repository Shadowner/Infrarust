use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::{CommandContext, CommandSource};
use infrarust_api::event::BoxFuture;

use crate::commands::actions::find_player;
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

            let message = match find_player(&*services.player_registry, target_name) {
                Ok(target) => match target.current_server() {
                    Some(server) => ProxyMessage::success(&format!(
                        "{target_name} is on server: {}",
                        server.as_str()
                    )),
                    None => ProxyMessage::info(&format!(
                        "{target_name} is online but not on any server."
                    )),
                },
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
