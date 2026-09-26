use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::Component;

use super::{INTERNAL_ERROR, verify_current_password};
use crate::handler::AuthHandler;
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
            let storage = self.handler.storage();
            let config = self.handler.config();

            let Some((username, _)) = verify_current_password(
                &self.handler,
                player.as_ref(),
                password,
                &config.messages.unregister_wrong_password,
            )
            .await
            else {
                return;
            };

            if let Err(e) = storage.delete_account(&username).await {
                tracing::error!("Account deletion error: {e}");
                let _ = player.send_message(Component::error(INTERNAL_ERROR));
                return;
            }
            let _ = player.send_message(parse_colored(&config.messages.unregister_success));
        })
    }
}
