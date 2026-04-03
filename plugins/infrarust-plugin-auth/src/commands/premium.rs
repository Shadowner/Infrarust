//! `/premium` command — disables cracked mode, re-enables premium auto-login.

use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;

use crate::account::Username;
use crate::handler::AuthHandler;
use crate::util::parse_colored;

pub struct PremiumCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for PremiumCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(sender_id) = ctx.player_id else {
                return;
            };
            let Some(player) = player_registry.get_player_by_id(sender_id) else {
                return;
            };

            let username = Username::new(&player.profile().username);
            let storage = self.handler.storage();
            let config = self.handler.config();

            let account = match storage.get_account(&username).await {
                Ok(Some(a)) => a,
                _ => {
                    let _ = player.send_message(Component::error("No account found."));
                    return;
                }
            };

            let is_cracked = account
                .premium_info
                .as_ref()
                .is_some_and(|pi| pi.force_cracked);

            if !is_cracked {
                let _ = player.send_message(Component::error("You are already in premium mode."));
                return;
            }

            let Some(mut premium_info) = account.premium_info else {
                let _ = player.send_message(Component::error(
                    "No premium record found. Reconnect with your official launcher first.",
                ));
                return;
            };
            premium_info.force_cracked = false;
            if let Err(e) = storage
                .update_premium_info(&username, Some(premium_info))
                .await
            {
                tracing::error!("Failed to unset force_cracked: {e}");
                let _ = player.send_message(Component::error("Internal error."));
                return;
            }

            if let Some(cache) = self.handler.premium_cache() {
                cache.invalidate(username.as_str());
            }

            let _ = player.send_message(parse_colored(&config.premium.messages.cracked_disabled));
        })
    }
}
