use std::sync::Arc;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::types::Component;

use super::INTERNAL_ERROR;
use crate::account::Username;
use crate::handler::AuthHandler;
use crate::util::parse_colored;

pub struct CrackedCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for CrackedCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(set_cracked_mode(&self.handler, ctx, true))
    }
}

pub struct PremiumCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for PremiumCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(set_cracked_mode(&self.handler, ctx, false))
    }
}

struct ModeMessages {
    already: &'static str,
    no_premium_record: &'static str,
    failure: &'static str,
}

const CRACKED: ModeMessages = ModeMessages {
    already: "You are already in cracked mode.",
    no_premium_record: "No premium record found. This command is only for premium players.",
    failure: "Failed to set force_cracked",
};

const PREMIUM: ModeMessages = ModeMessages {
    already: "You are already in premium mode.",
    no_premium_record: "No premium record found. Reconnect with your official launcher first.",
    failure: "Failed to unset force_cracked",
};

async fn set_cracked_mode(handler: &AuthHandler, ctx: CommandContext, cracked: bool) {
    let Some(player) = ctx.source.player() else {
        ctx.source
            .send_message(Component::error("Only players can use this command."));
        return;
    };

    let messages = if cracked { CRACKED } else { PREMIUM };
    let username = Username::new(&player.profile().username);
    let storage = handler.storage();
    let config = handler.config();

    let account = match storage.get_account(&username) {
        Ok(Some(a)) => a,
        _ => {
            let _ = player.send_message(Component::error("No account found."));
            return;
        }
    };

    let force_cracked = account
        .premium_info
        .as_ref()
        .is_some_and(|pi| pi.force_cracked);
    if force_cracked == cracked {
        let _ = player.send_message(Component::error(messages.already));
        return;
    }

    let Some(mut premium_info) = account.premium_info else {
        let _ = player.send_message(Component::error(messages.no_premium_record));
        return;
    };
    premium_info.force_cracked = cracked;

    if let Err(e) = storage
        .update_premium_info(&username, Some(premium_info))
        .await
    {
        tracing::error!("{}: {e}", messages.failure);
        let _ = player.send_message(Component::error(INTERNAL_ERROR));
        return;
    }

    if let Some(cache) = handler.premium_cache() {
        cache.invalidate(username.as_str());
    }

    let confirmation = if cracked {
        &config.premium.messages.cracked_enabled
    } else {
        &config.premium.messages.cracked_disabled
    };
    let _ = player.send_message(parse_colored(confirmation));
}
