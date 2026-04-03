use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;

use crate::account::Username;
use crate::handler::AuthHandler;
use crate::password;
use crate::util::parse_colored;

pub struct ChangePasswordCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ChangePasswordCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player_id) = ctx.player_id else {
                return;
            };
            let Some(player) = player_registry.get_player_by_id(player_id) else {
                return;
            };

            if ctx.args.len() < 2 {
                let _ = player.send_message(parse_colored(
                    &self.handler.config().messages.changepassword_usage,
                ));
                return;
            }

            let old_password = &ctx.args[0];
            let new_password = &ctx.args[1];
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

            let Some(ref password_hash) = account.password_hash else {
                let _ = player.send_message(Component::error(
                    "This is a premium account with no password set.",
                ));
                return;
            };

            match password::verify_password(old_password, password_hash).await {
                Ok(true) => {}
                Ok(false) => {
                    let _ = player
                        .send_message(parse_colored(&config.messages.changepassword_wrong_old));
                    return;
                }
                Err(e) => {
                    tracing::error!("Password verification error: {e}");
                    let _ = player.send_message(Component::error("Internal error."));
                    return;
                }
            }

            match password::hash_password(new_password, &config.hashing).await {
                Ok(new_hash) => {
                    if let Err(e) = storage.update_password_hash(&username, new_hash).await {
                        tracing::error!("Password update error: {e}");
                        let _ = player.send_message(Component::error("Internal error."));
                        return;
                    }
                    let _ =
                        player.send_message(parse_colored(&config.messages.changepassword_success));
                }
                Err(e) => {
                    tracing::error!("Password hashing error: {e}");
                    let _ = player.send_message(Component::error("Internal error."));
                }
            }
        })
    }
}
