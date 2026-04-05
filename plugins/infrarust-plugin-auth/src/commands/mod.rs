pub mod changepassword;
pub mod cracked;
pub mod forcechangepassword;
pub mod forcelogin;
pub mod forceunregister;
pub mod premium;
pub mod unregister;

use std::sync::Arc;

use infrarust_api::plugin::PluginContext;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::PlayerId;

use crate::handler::AuthHandler;

pub fn register_commands(ctx: &dyn PluginContext, handler: Arc<AuthHandler>) {
    ctx.command_manager().register(
        "changepassword",
        &["changepw", "cp"],
        "Change your auth password",
        Box::new(changepassword::ChangePasswordCommand {
            handler: Arc::clone(&handler),
        }),
    );

    ctx.command_manager().register(
        "unregister",
        &[],
        "Delete your auth account",
        Box::new(unregister::UnregisterCommand {
            handler: Arc::clone(&handler),
        }),
    );

    ctx.command_manager().register(
        "forcelogin",
        &[],
        "Force-authenticate a player in auth limbo",
        Box::new(forcelogin::ForceLoginCommand {
            handler: Arc::clone(&handler),
        }),
    );

    ctx.command_manager().register(
        "forceunregister",
        &[],
        "Delete another player's auth account",
        Box::new(forceunregister::ForceUnregisterCommand {
            handler: Arc::clone(&handler),
        }),
    );

    ctx.command_manager().register(
        "forcechangepassword",
        &[],
        "Change another player's password",
        Box::new(forcechangepassword::ForceChangePasswordCommand {
            handler: Arc::clone(&handler),
        }),
    );

    if handler.config().premium.enabled && handler.config().premium.allow_cracked_command {
        ctx.command_manager().register(
            "cracked",
            &[],
            "Force cracked mode (use /login instead of premium auto-login)",
            Box::new(cracked::CrackedCommand {
                handler: Arc::clone(&handler),
            }),
        );

        ctx.command_manager().register(
            "premium",
            &[],
            "Re-enable premium auto-login",
            Box::new(premium::PremiumCommand {
                handler: Arc::clone(&handler),
            }),
        );
    }
}

fn is_admin(player_id: PlayerId, player_registry: &dyn PlayerRegistry) -> bool {
    player_registry
        .get_player_by_id(player_id)
        .is_some_and(|p| p.has_permission("infrarust.admin"))
}
