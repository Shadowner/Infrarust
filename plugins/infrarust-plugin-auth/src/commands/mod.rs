pub mod changepassword;
pub mod cracked_mode;
pub mod forcechangepassword;
pub mod forcelogin;
pub mod forceunregister;
#[cfg(test)]
mod tests;
pub mod unregister;

use std::sync::Arc;

use infrarust_api::command::{CommandHandler, CommandSource, CommandSpec};
use infrarust_api::player::Player;
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::Component;

use crate::account::{AuthAccount, Username};
use crate::handler::{Attempt, AuthHandler};
use crate::password;
use crate::util::parse_colored;

pub(crate) const INTERNAL_ERROR: &str = "Internal error.";
const ADMIN_PERMISSION: &str = "infrarust.admin";
pub(crate) const NOT_LOGGED_IN: &str = "You must be logged in to use this command.";

pub(crate) fn authenticated_player<'a>(
    handler: &AuthHandler,
    source: &'a CommandSource,
) -> Option<&'a Arc<dyn Player>> {
    let Some(player) = source.player() else {
        source.send_message(Component::error("Only players can use this command."));
        return None;
    };
    if !handler.is_authenticated(player.id()) {
        let _ = player.send_message(Component::error(NOT_LOGGED_IN));
        return None;
    }
    Some(player)
}

pub(crate) async fn verify_current_password(
    handler: &AuthHandler,
    player: &dyn Player,
    password: &str,
    wrong_password: &str,
) -> Option<Username> {
    let username = Username::new(&player.profile().username);

    let stored = match handler.storage().get_account(&username) {
        Ok(Some(AuthAccount {
            password_hash: Some(hash),
            ..
        })) => Some(hash),
        _ => None,
    };
    let hash = stored.as_ref().unwrap_or_else(|| handler.dummy_hash());

    match password::verify_password(password, hash).await {
        Ok(true) if stored.is_some() => {
            handler.clear_failed_attempts(player.id());
            Some(username)
        }
        Ok(_) => {
            match handler.record_failed_attempt(player.id()) {
                Attempt::Exhausted => {
                    let config = handler.config();
                    player
                        .disconnect(parse_colored(&config.messages.login_max_attempts))
                        .await;
                }
                Attempt::Retry { .. } => {
                    let _ = player.send_message(parse_colored(wrong_password));
                }
            }
            None
        }
        Err(e) => {
            tracing::error!("Password verification error: {e}");
            let _ = player.send_message(Component::error(INTERNAL_ERROR));
            None
        }
    }
}

pub fn register_commands(ctx: &dyn PluginContext, handler: Arc<AuthHandler>) {
    let commands = ctx.command_manager();
    let mut specs: Vec<(CommandSpec, Box<dyn CommandHandler>)> = vec![
        (
            CommandSpec::new("changepassword")
                .aliases(["changepw", "cp"])
                .description("Change your auth password")
                .usage("/changepassword <old> <new>"),
            Box::new(changepassword::ChangePasswordCommand {
                handler: Arc::clone(&handler),
            }),
        ),
        (
            CommandSpec::new("unregister")
                .description("Delete your auth account")
                .usage("/unregister <password>"),
            Box::new(unregister::UnregisterCommand {
                handler: Arc::clone(&handler),
            }),
        ),
        (
            CommandSpec::new("forcelogin")
                .description("Force-authenticate a player in auth limbo")
                .usage("/forcelogin <username>"),
            Box::new(forcelogin::ForceLoginCommand {
                handler: Arc::clone(&handler),
            }),
        ),
        (
            CommandSpec::new("forceunregister")
                .description("Delete another player's auth account")
                .usage("/forceunregister <username>"),
            Box::new(forceunregister::ForceUnregisterCommand {
                handler: Arc::clone(&handler),
            }),
        ),
        (
            CommandSpec::new("forcechangepassword")
                .description("Change another player's password")
                .usage("/forcechangepassword <username> <password>"),
            Box::new(forcechangepassword::ForceChangePasswordCommand {
                handler: Arc::clone(&handler),
            }),
        ),
    ];

    if handler.config().premium.enabled && handler.config().premium.allow_cracked_command {
        specs.push((
            CommandSpec::new("cracked")
                .description("Force cracked mode (use /login instead of premium auto-login)"),
            Box::new(cracked_mode::CrackedCommand {
                handler: Arc::clone(&handler),
            }),
        ));
        specs.push((
            CommandSpec::new("premium").description("Re-enable premium auto-login"),
            Box::new(cracked_mode::PremiumCommand {
                handler: Arc::clone(&handler),
            }),
        ));
    }

    for (spec, command) in specs {
        let name = spec.name.clone();
        match commands.register(spec, command) {
            Ok(registration) if !registration.rejected_aliases.is_empty() => {
                tracing::warn!(
                    "[auth] /{name}: aliases {:?} are taken and were skipped",
                    registration.rejected_aliases
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("[auth] /{name} was not registered: {e}"),
        }
    }
}

fn is_admin(source: &CommandSource, handler: &AuthHandler) -> bool {
    let Some(player) = source.player() else {
        return source.has_permission(ADMIN_PERMISSION);
    };
    if handler.is_in_auth_limbo(player.id()) {
        return false;
    }
    player.has_permission(ADMIN_PERMISSION)
        || (handler.is_authenticated(player.id())
            && handler
                .config()
                .admin_set()
                .contains(&player.profile().username.to_lowercase()))
}
