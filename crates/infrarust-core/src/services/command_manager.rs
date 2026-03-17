//! [`CommandManager`] implementation.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use infrarust_api::command::{CommandContext, CommandHandler, CommandManager};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::PlayerId;

/// A registered command with its handler and metadata.
struct RegisteredCommand {
    handler: Arc<dyn CommandHandler>,
    aliases: Vec<String>,
    #[allow(dead_code)]
    description: String,
}

/// Concrete [`CommandManager`] implementation.
///
/// Commands and aliases are stored in lowercase for case-insensitive lookup.
pub struct CommandManagerImpl {
    commands: RwLock<HashMap<String, RegisteredCommand>>,
    aliases: RwLock<HashMap<String, String>>,
}

impl CommandManagerImpl {
    /// Creates a new empty command manager.
    pub fn new() -> Self {
        Self {
            commands: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashMap::new()),
        }
    }

    /// Dispatches a command input string.
    ///
    /// Returns `true` if the command was found and executed (should not be
    /// forwarded to the backend), `false` otherwise.
    pub async fn dispatch(
        &self,
        player_id: Option<PlayerId>,
        input: &str,
        player_registry: &dyn PlayerRegistry,
    ) -> bool {
        let input = input.trim();
        let (name, args_str) = match input.split_once(' ') {
            Some((n, a)) => (n, a),
            None => (input, ""),
        };

        let name_lower = name.to_lowercase();

        // Resolve alias → canonical name
        let canonical = {
            let aliases = self.aliases.read().unwrap_or_else(|e| e.into_inner());
            aliases.get(&name_lower).cloned().unwrap_or(name_lower)
        };

        // Look up handler
        let handler_exists = {
            let commands = self.commands.read().unwrap_or_else(|e| e.into_inner());
            commands.contains_key(&canonical)
        };

        if !handler_exists {
            return false;
        }

        let args: Vec<String> = if args_str.is_empty() {
            vec![]
        } else {
            args_str.split_whitespace().map(String::from).collect()
        };

        let ctx = CommandContext {
            player_id,
            args,
            raw: input.to_string(),
        };

        // Clone the handler Arc so we can drop the lock before awaiting.
        let handler = {
            let commands = self.commands.read().unwrap_or_else(|e| e.into_inner());
            commands.get(&canonical).map(|cmd| Arc::clone(&cmd.handler))
        };

        match handler {
            Some(handler) => {
                handler.execute(ctx, player_registry).await;
                true
            }
            None => false,
        }
    }
}

impl Default for CommandManagerImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl infrarust_api::command::private::Sealed for CommandManagerImpl {}

impl CommandManager for CommandManagerImpl {
    fn register(
        &self,
        name: &str,
        aliases: &[&str],
        description: &str,
        handler: Box<dyn CommandHandler>,
    ) {
        let name_lower = name.to_lowercase();

        // Clean up any previous registration (removes orphaned aliases).
        self.unregister(name);

        let alias_list: Vec<String> = aliases.iter().map(|a| a.to_lowercase()).collect();

        // Register aliases
        {
            let mut alias_map = self.aliases.write().unwrap_or_else(|e| e.into_inner());
            for alias in &alias_list {
                alias_map.insert(alias.clone(), name_lower.clone());
            }
        }

        // Register command
        {
            let mut commands = self.commands.write().unwrap_or_else(|e| e.into_inner());
            commands.insert(
                name_lower,
                RegisteredCommand {
                    handler: Arc::from(handler),
                    aliases: alias_list,
                    description: description.to_string(),
                },
            );
        }
    }

    fn unregister(&self, name: &str) {
        let name_lower = name.to_lowercase();

        // Remove aliases first
        let alias_list = {
            let commands = self.commands.read().unwrap_or_else(|e| e.into_inner());
            commands
                .get(&name_lower)
                .map(|cmd| cmd.aliases.clone())
                .unwrap_or_default()
        };

        {
            let mut alias_map = self.aliases.write().unwrap_or_else(|e| e.into_inner());
            for alias in &alias_list {
                alias_map.remove(alias);
            }
        }

        // Remove command
        {
            let mut commands = self.commands.write().unwrap_or_else(|e| e.into_inner());
            commands.remove(&name_lower);
        }
    }
}
