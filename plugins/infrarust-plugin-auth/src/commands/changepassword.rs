use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::Component;

use super::{INTERNAL_ERROR, authenticated_player, verify_current_password};
use crate::handler::AuthHandler;
use crate::password;
use crate::util::parse_colored;

pub struct ChangePasswordCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ChangePasswordCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player) = authenticated_player(&self.handler, &ctx.source) else {
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
            let storage = self.handler.storage();
            let config = self.handler.config();

            let Some(username) = verify_current_password(
                &self.handler,
                player.as_ref(),
                old_password,
                &config.messages.changepassword_wrong_old,
            )
            .await
            else {
                return;
            };

            match password::hash_password(new_password, &config.hashing).await {
                Ok(new_hash) => {
                    if let Err(e) = storage.update_password_hash(&username, new_hash).await {
                        tracing::error!("Password update error: {e}");
                        let _ = player.send_message(Component::error(INTERNAL_ERROR));
                        return;
                    }
                    let _ =
                        player.send_message(parse_colored(&config.messages.changepassword_success));
                }
                Err(e) => {
                    tracing::error!("Password hashing error: {e}");
                    let _ = player.send_message(Component::error(INTERNAL_ERROR));
                }
            }
        })
    }
}
