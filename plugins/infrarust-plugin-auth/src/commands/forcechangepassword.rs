use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::Component;

use crate::account::Username;
use crate::handler::AuthHandler;
use crate::password;
use crate::util::parse_colored;

pub struct ForceChangePasswordCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ForceChangePasswordCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !super::is_admin(&ctx.source, self.handler.config()) {
                ctx.source.send_message(parse_colored(
                    &self.handler.config().messages.admin_no_permission,
                ));
                return;
            }

            let config = self.handler.config();

            if ctx.args.len() < 2 {
                ctx.source
                    .send_message(parse_colored(&config.messages.forcechangepassword_usage));
                return;
            }

            let target_name = &ctx.args[0];
            let new_password = &ctx.args[1];
            let username = Username::new(target_name);
            let storage = self.handler.storage();

            match storage.has_account(&username).await {
                Ok(true) => {}
                Ok(false) => {
                    let msg = config.messages.format_message(
                        &config.messages.forcechangepassword_not_found,
                        &[("{username}", target_name)],
                    );
                    ctx.source.send_message(parse_colored(&msg));
                    return;
                }
                Err(e) => {
                    tracing::error!("Storage error: {e}");
                    ctx.source.send_message(Component::error("Internal error."));
                    return;
                }
            }

            match password::hash_password(new_password, &config.hashing).await {
                Ok(new_hash) => {
                    if let Err(e) = storage.update_password_hash(&username, new_hash).await {
                        tracing::error!("Password update error: {e}");
                        ctx.source.send_message(Component::error("Internal error."));
                        return;
                    }
                    let msg = config.messages.format_message(
                        &config.messages.forcechangepassword_success,
                        &[("{username}", target_name)],
                    );
                    ctx.source.send_message(parse_colored(&msg));
                }
                Err(e) => {
                    tracing::error!("Password hashing error: {e}");
                    ctx.source.send_message(Component::error("Internal error."));
                }
            }
        })
    }
}
