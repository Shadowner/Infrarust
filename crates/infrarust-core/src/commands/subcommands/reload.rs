use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::services::player_registry::PlayerRegistry;

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
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player_id) = ctx.player_id else {
                return;
            };
            let Some(player) = services.player_registry.get_player_by_id(player_id) else {
                return;
            };

            let _ = player.send_message(ProxyMessage::info(
                "Configuration auto-reloads when files change.",
            ));
            let _ = player.send_message(ProxyMessage::detail("  No manual reload is needed."));
        })
    }
}
