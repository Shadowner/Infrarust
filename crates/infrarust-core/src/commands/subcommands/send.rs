use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::{CommandContext, CommandSource};
use infrarust_api::event::BoxFuture;

use crate::commands::actions::send_player;
use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct SendSubcommand;

impl SubcommandHandler for SendSubcommand {
    fn name(&self) -> &str {
        "send"
    }

    fn description(&self) -> &str {
        "Send a player to a server"
    }

    fn admin_only(&self) -> bool {
        true
    }

    fn usage(&self) -> &str {
        "/ir send <player> <server>"
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let sender = &ctx.source;

            let [target_name, server_name, ..] = args else {
                sender.send_message(ProxyMessage::error("Usage: /ir send <player> <server>"));
                return;
            };

            let message = match send_player(
                &*services.player_registry,
                &*services.config_service,
                target_name,
                server_name,
            )
            .await
            {
                Ok(sent) => ProxyMessage::success(&format!(
                    "Sending {} to '{}'...",
                    sent.player,
                    sent.server.as_str()
                )),
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
                [_, prefix] => services.complete_server_names(prefix),
                _ => Vec::new(),
            }
        })
    }
}
