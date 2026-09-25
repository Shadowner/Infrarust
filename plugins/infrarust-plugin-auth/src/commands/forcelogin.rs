use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::Component;

use crate::handler::AuthHandler;
use crate::util::parse_colored;

pub struct ForceLoginCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ForceLoginCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let config = self.handler.config();
            if !super::is_admin(&ctx.source, config) {
                ctx.source
                    .send_message(parse_colored(&config.messages.admin_no_permission));
                return;
            }

            let Some(target_name) = ctx.args.first() else {
                ctx.source
                    .send_message(Component::error("Usage: /forcelogin <username>"));
                return;
            };

            let Some(target_player) = self.handler.player_registry().get_player(target_name) else {
                let msg = config.messages.format_message(
                    &config.messages.forcelogin_not_found,
                    &[("{username}", target_name)],
                );
                ctx.source.send_message(parse_colored(&msg));
                return;
            };

            if self.handler.force_complete_session(target_player.id()) {
                let _ = target_player.send_message(
                    Component::text("An admin has authenticated you. Type anything to continue.")
                        .color("green"),
                );
                let msg = config.messages.format_message(
                    &config.messages.forcelogin_success,
                    &[("{username}", target_name)],
                );
                ctx.source.send_message(parse_colored(&msg));
            } else {
                let msg = config.messages.format_message(
                    &config.messages.forcelogin_not_in_limbo,
                    &[("{username}", target_name)],
                );
                ctx.source.send_message(parse_colored(&msg));
            }
        })
    }
}
