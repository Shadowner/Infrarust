use std::collections::HashMap;

use infrarust_api::command::CommandContext;
use infrarust_api::message::ProxyMessage;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) fn handle_help(
    ctx: &CommandContext,
    args: &[String],
    subcommands: &HashMap<String, Box<dyn SubcommandHandler>>,
    services: &CommandServices,
) {
    let player = &ctx.source;

    if let Some(cmd_name) = args.first() {
        let lower = cmd_name.to_lowercase();
        if let Some(sub) = subcommands.get(&lower) {
            if services
                .permission_service
                .is_command_allowed(&lower, player)
            {
                player.send_message(ProxyMessage::info(&format!(
                    "{} — {}",
                    sub.usage(),
                    sub.description()
                )));
            } else {
                player.send_message(ProxyMessage::error(crate::commands::NO_PERMISSION));
            }
        } else {
            player.send_message(ProxyMessage::error(&format!(
                "Unknown command: '{cmd_name}'. Use /ir help for a list."
            )));
        }
    } else {
        player.send_message(ProxyMessage::info("Available commands:"));

        let mut names: Vec<&String> = subcommands.keys().collect();
        names.sort();

        for name in names {
            if let Some(sub) = subcommands.get(name)
                && services.permission_service.is_command_allowed(name, player)
            {
                player.send_message(ProxyMessage::detail(&format!(
                    "  {:<12} - {}",
                    sub.name(),
                    sub.description()
                )));
            }
        }

        player.send_message(ProxyMessage::detail("Use /ir help <command> for details."));
    }
}

pub(crate) struct HelpSubcommand;

impl SubcommandHandler for HelpSubcommand {
    fn name(&self) -> &str {
        "help"
    }

    fn description(&self) -> &str {
        "Show help for proxy commands"
    }

    fn usage(&self) -> &str {
        "/ir help [command]"
    }

    fn execute<'a>(
        &'a self,
        _ctx: &'a CommandContext,
        _args: &'a [String],
        _services: &'a CommandServices,
    ) -> infrarust_api::event::BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}
