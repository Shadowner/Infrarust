pub mod changepassword;
pub mod cracked;
pub mod forcechangepassword;
pub mod forcelogin;
pub mod forceunregister;
pub mod premium;
#[cfg(test)]
mod tests;
pub mod unregister;

use std::sync::Arc;

use infrarust_api::command::{CommandHandler, CommandSource, CommandSpec};
use infrarust_api::plugin::PluginContext;

use crate::config::AuthConfig;
use crate::handler::AuthHandler;

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
            Box::new(cracked::CrackedCommand {
                handler: Arc::clone(&handler),
            }),
        ));
        specs.push((
            CommandSpec::new("premium").description("Re-enable premium auto-login"),
            Box::new(premium::PremiumCommand {
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

fn is_admin(source: &CommandSource, config: &AuthConfig) -> bool {
    source.has_permission("infrarust.admin")
        || source.player().is_some_and(|player| {
            config
                .admin_set()
                .contains(&player.profile().username.to_lowercase())
        })
}
