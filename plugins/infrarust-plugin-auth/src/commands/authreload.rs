use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::player_registry::PlayerRegistry;

use crate::handler::AuthHandler;
use crate::util::parse_colored;

pub struct AuthReloadCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for AuthReloadCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(sender_id) = ctx.player_id else {
                return;
            };

            if !super::is_admin(sender_id, player_registry, &self.handler) {
                if let Some(player) = player_registry.get_player_by_id(sender_id) {
                    let _ = player.send_message(parse_colored(
                        &self.handler.config().messages.admin_no_permission,
                    ));
                }
                return;
            }

            if let Some(player) = player_registry.get_player_by_id(sender_id) {
                let _ = player.send_message(parse_colored(
                    &self.handler.config().messages.authreload_success,
                ));
            }
        })
    }
}
