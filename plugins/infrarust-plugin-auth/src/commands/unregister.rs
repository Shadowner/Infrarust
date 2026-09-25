use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::Component;

use crate::account::Username;
use crate::handler::AuthHandler;
use crate::password;
use crate::util::parse_colored;

pub struct UnregisterCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for UnregisterCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player) = ctx.source.player() else {
                ctx.source
                    .send_message(Component::error("Only players can use this command."));
                return;
            };

            if ctx.args.is_empty() {
                let _ = player.send_message(parse_colored(
                    &self.handler.config().messages.unregister_usage,
                ));
                return;
            }

            let password = &ctx.args[0];
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

            match password::verify_password(password, password_hash).await {
                Ok(true) => {
                    if let Err(e) = storage.delete_account(&username).await {
                        tracing::error!("Account deletion error: {e}");
                        let _ = player.send_message(Component::error("Internal error."));
                        return;
                    }
                    let _ = player.send_message(parse_colored(&config.messages.unregister_success));
                }
                Ok(false) => {
                    let _ = player
                        .send_message(parse_colored(&config.messages.unregister_wrong_password));
                }
                Err(e) => {
                    tracing::error!("Password verification error: {e}");
                    let _ = player.send_message(Component::error("Internal error."));
                }
            }
        })
    }
}
