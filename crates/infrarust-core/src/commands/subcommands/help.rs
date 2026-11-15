use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::CommandContext;

use crate::commands::{ArgSpec, CommandServices, SUBCOMMANDS, SubcommandHandler};

pub(crate) fn handle_help(ctx: &CommandContext, args: &[String], services: &CommandServices) {
    let player = &ctx.source;

    if let Some(cmd_name) = args.first() {
        let lower = cmd_name.to_lowercase();
        if let Some(entry) = SUBCOMMANDS.entry(&lower) {
            if services
                .permission_service
                .is_command_allowed(&lower, player)
            {
                player.send_message(ProxyMessage::info(&format!(
                    "{} — {}",
                    entry.usage, entry.description
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

        let mut entries: Vec<_> = SUBCOMMANDS.entries().collect();
        entries.sort_by_key(|entry| entry.name);

        for entry in entries {
            if services
                .permission_service
                .is_command_allowed(entry.name, player)
            {
                player.send_message(ProxyMessage::detail(&format!(
                    "  {:<12} - {}",
                    entry.name, entry.description
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

    fn arguments(&self) -> &'static [ArgSpec] {
        const { &[ArgSpec::word("command")] }
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> infrarust_api::event::BoxFuture<'a, ()> {
        Box::pin(async move { handle_help(ctx, args, services) })
    }
}
