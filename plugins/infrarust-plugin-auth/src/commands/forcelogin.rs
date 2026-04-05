use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;

use crate::handler::AuthHandler;
use crate::util::parse_colored;

pub struct ForceLoginCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ForceLoginCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(sender_id) = ctx.player_id else {
                return;
            };

            if !super::is_admin(sender_id, player_registry) {
                if let Some(player) = player_registry.get_player_by_id(sender_id) {
                    let _ = player.send_message(parse_colored(
                        &self.handler.config().messages.admin_no_permission,
                    ));
                }
                return;
            }

            let Some(target_name) = ctx.args.first() else {
                if let Some(player) = player_registry.get_player_by_id(sender_id) {
                    let _ = player.send_message(Component::error("Usage: /forcelogin <username>"));
                }
                return;
            };

            let Some(target_player) = player_registry.get_player(target_name) else {
                if let Some(player) = player_registry.get_player_by_id(sender_id) {
                    let msg = self.handler.config().messages.format_message(
                        &self.handler.config().messages.forcelogin_not_found,
                        &[("{username}", target_name)],
                    );
                    let _ = player.send_message(parse_colored(&msg));
                }
                return;
            };

            let target_id = target_player.id();
            let config = self.handler.config();

            if self.handler.force_complete_session(target_id) {
                let _ = target_player.send_message(
                    Component::text("An admin has authenticated you. Type anything to continue.")
                        .color("green"),
                );

                if let Some(admin) = player_registry.get_player_by_id(sender_id) {
                    let msg = config.messages.format_message(
                        &config.messages.forcelogin_success,
                        &[("{username}", target_name)],
                    );
                    let _ = admin.send_message(parse_colored(&msg));
                }
            } else if let Some(admin) = player_registry.get_player_by_id(sender_id) {
                let msg = config.messages.format_message(
                    &config.messages.forcelogin_not_in_limbo,
                    &[("{username}", target_name)],
                );
                let _ = admin.send_message(parse_colored(&msg));
            }
        })
    }
}
