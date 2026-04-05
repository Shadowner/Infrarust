//! Command system.
//!
//! Plugins register commands via [`CommandManager`] and implement
//! [`CommandHandler`] to handle execution and tab-completion.

use crate::event::BoxFuture;
use crate::services::player_registry::PlayerRegistry;
use crate::types::PlayerId;

/// Context provided to a command handler during execution.
#[derive(Debug)]
pub struct CommandContext {
    /// The player who executed the command, if any (console commands have no player).
    pub player_id: Option<PlayerId>,
    /// The command arguments (split by whitespace).
    pub args: Vec<String>,
    /// The full command string as typed.
    pub raw: String,
}

/// A handler for a registered command.
///
/// Methods use [`BoxFuture`] to allow dyn-dispatch (`Box<dyn CommandHandler>`).
/// Implement by returning `Box::pin(async move { ... })`.
///
/// # Example
/// ```ignore
/// use infrarust_api::prelude::*;
///
/// struct PingCommand;
///
/// impl CommandHandler for PingCommand {
///     fn execute<'a>(&'a self, ctx: CommandContext, players: &'a dyn PlayerRegistry) -> BoxFuture<'a, ()> {
///         Box::pin(async move {
///             if let Some(id) = ctx.player_id {
///                 if let Some(player) = players.get_player_by_id(id) {
///                     player.send_message(Component::text("Pong!").color("green")).ok();
///                 }
///             }
///         })
///     }
/// }
/// ```
pub trait CommandHandler: Send + Sync {
    /// Executes the command.
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()>;

    /// Returns tab-completion suggestions for partial arguments.
    ///
    /// The default implementation returns no suggestions.
    fn tab_complete(&self, _partial_args: &[&str]) -> Vec<String> {
        Vec::new()
    }
}

pub mod private {
    /// Sealed — only the proxy implements [`CommandManager`](super::CommandManager).
    pub trait Sealed {}
}

/// Service for registering and unregistering commands.
///
/// Obtained via [`PluginContext::command_manager()`](crate::plugin::PluginContext::command_manager).
pub trait CommandManager: Send + Sync + private::Sealed {
    /// Registers a command with the given name, aliases, description, and handler.
    fn register(
        &self,
        name: &str,
        aliases: &[&str],
        description: &str,
        handler: Box<dyn CommandHandler>,
    );

    /// Registers a command associated with a specific plugin.
    ///
    /// The default implementation ignores `plugin_id` and delegates to [`register`](Self::register).
    fn register_with_plugin_id(
        &self,
        name: &str,
        aliases: &[&str],
        description: &str,
        handler: Box<dyn CommandHandler>,
        plugin_id: &str,
    ) {
        let _ = plugin_id;
        self.register(name, aliases, description, handler);
    }

    /// Unregisters a command by name.
    fn unregister(&self, name: &str);
}
