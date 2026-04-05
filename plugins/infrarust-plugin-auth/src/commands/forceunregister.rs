use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;

use crate::account::Username;
use crate::handler::AuthHandler;
use crate::util::parse_colored;

pub struct ForceUnregisterCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ForceUnregisterCommand {
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
                    let _ =
                        player.send_message(Component::error("Usage: /forceunregister <username>"));
                }
                return;
            };

            let username = Username::new(target_name);
            let storage = self.handler.storage();
            let config = self.handler.config();

            match storage.delete_account(&username).await {
                Ok(true) => {
                    if let Some(player) = player_registry.get_player_by_id(sender_id) {
                        let msg = config.messages.format_message(
                            &config.messages.forceunregister_success,
                            &[("{username}", target_name)],
                        );
                        let _ = player.send_message(parse_colored(&msg));
                    }
                }
                Ok(false) => {
                    if let Some(player) = player_registry.get_player_by_id(sender_id) {
                        let msg = config.messages.format_message(
                            &config.messages.forceunregister_not_found,
                            &[("{username}", target_name)],
                        );
                        let _ = player.send_message(parse_colored(&msg));
                    }
                }
                Err(e) => {
                    tracing::error!("Force unregister error: {e}");
                    if let Some(player) = player_registry.get_player_by_id(sender_id) {
                        let _ = player.send_message(Component::error("Internal error."));
                    }
                }
            }
        })
    }
}
