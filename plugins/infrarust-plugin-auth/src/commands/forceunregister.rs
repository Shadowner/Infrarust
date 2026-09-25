use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::Component;

use crate::account::Username;
use crate::handler::AuthHandler;
use crate::util::parse_colored;

pub struct ForceUnregisterCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ForceUnregisterCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !super::is_admin(&ctx.source, self.handler.config()) {
                ctx.source.send_message(parse_colored(
                    &self.handler.config().messages.admin_no_permission,
                ));
                return;
            }

            let Some(target_name) = ctx.args.first() else {
                ctx.source
                    .send_message(Component::error("Usage: /forceunregister <username>"));
                return;
            };

            let username = Username::new(target_name);
            let storage = self.handler.storage();
            let config = self.handler.config();

            match storage.delete_account(&username).await {
                Ok(true) => {
                    let msg = config.messages.format_message(
                        &config.messages.forceunregister_success,
                        &[("{username}", target_name)],
                    );
                    ctx.source.send_message(parse_colored(&msg));
                }
                Ok(false) => {
                    let msg = config.messages.format_message(
                        &config.messages.forceunregister_not_found,
                        &[("{username}", target_name)],
                    );
                    ctx.source.send_message(parse_colored(&msg));
                }
                Err(e) => {
                    tracing::error!("Force unregister error: {e}");
                    ctx.source.send_message(Component::error("Internal error."));
                }
            }
        })
    }
}
