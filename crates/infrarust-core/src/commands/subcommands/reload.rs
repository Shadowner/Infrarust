use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct ReloadSubcommand;

impl SubcommandHandler for ReloadSubcommand {
    fn name(&self) -> &str {
        "reload"
    }

    fn description(&self) -> &str {
        "Configuration reload info"
    }

    fn required_level(&self) -> PermissionLevel {
        PermissionLevel::Admin
    }

    fn usage(&self) -> &str {
        "/ir reload"
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        _args: &'a [String],
        _services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let player = &ctx.source;

            player.send_message(ProxyMessage::info(
                "Configuration auto-reloads when files change.",
            ));
            player.send_message(ProxyMessage::detail("  No manual reload is needed."));
        })
    }
}
