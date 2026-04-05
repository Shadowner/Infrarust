use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use infrarust_api::command::{CommandContext, CommandHandler, CommandManager};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::PlayerId;

struct RegisteredCommand {
    handler: Arc<dyn CommandHandler>,
    aliases: Vec<String>,
    description: String,
    is_builtin: bool,
    plugin_id: String,
}

pub struct CommandManagerImpl {
    commands: RwLock<HashMap<String, RegisteredCommand>>,
    aliases: RwLock<HashMap<String, String>>,
}

impl CommandManagerImpl {
    pub fn new() -> Self {
        Self {
            commands: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashMap::new()),
        }
    }

    /// Returns `true` if the command was found and executed.
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

        let canonical = {
            let aliases = self.aliases.read().expect("lock poisoned");
            aliases.get(&name_lower).cloned().unwrap_or(name_lower)
        };

        let handler_exists = {
            let commands = self.commands.read().expect("lock poisoned");
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

        let handler = {
            let commands = self.commands.read().expect("lock poisoned");
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

    pub fn register_builtin(
        &self,
        name: &str,
        aliases: &[&str],
        description: &str,
        handler: Box<dyn CommandHandler>,
    ) {
        let name_lower = name.to_lowercase();
        let alias_list: Vec<String> = aliases.iter().map(|a| a.to_lowercase()).collect();

        {
            let mut alias_map = self.aliases.write().expect("lock poisoned");
            for alias in &alias_list {
                alias_map.insert(alias.clone(), name_lower.clone());
            }
        }

        {
            let mut commands = self.commands.write().expect("lock poisoned");
            commands.insert(
                name_lower,
                RegisteredCommand {
                    handler: Arc::from(handler),
                    aliases: alias_list,
                    description: description.to_string(),
                    is_builtin: true,
                    plugin_id: String::new(),
                },
            );
        }
    }

    pub fn list_commands(&self) -> Vec<(String, String)> {
        let commands = self.commands.read().expect("lock poisoned");
        commands
            .iter()
            .map(|(name, cmd)| (name.clone(), cmd.description.clone()))
            .collect()
    }

    pub fn list_plugin_commands(&self) -> Vec<(String, String)> {
        let commands = self.commands.read().expect("lock poisoned");
        commands
            .iter()
            .filter(|(_, cmd)| !cmd.is_builtin)
            .map(|(name, cmd)| (name.clone(), cmd.plugin_id.clone()))
            .collect()
    }

    pub fn find_plugin_command(
        &self,
        plugin_id: &str,
        command_name: &str,
    ) -> Option<Arc<dyn CommandHandler>> {
        let name_lower = command_name.to_lowercase();
        let commands = self.commands.read().expect("lock poisoned");
        commands
            .get(&name_lower)
            .filter(|cmd| cmd.plugin_id == plugin_id)
            .map(|cmd| Arc::clone(&cmd.handler))
    }

    pub fn commands_for_plugin(&self, plugin_id: &str) -> Vec<(String, String)> {
        let commands = self.commands.read().expect("lock poisoned");
        commands
            .iter()
            .filter(|(_, cmd)| cmd.plugin_id == plugin_id)
            .map(|(name, cmd)| (name.clone(), cmd.description.clone()))
            .collect()
    }

    pub fn is_plugin_command(&self, name: &str) -> bool {
        let name_lower = name.to_lowercase();
        let commands = self.commands.read().expect("lock poisoned");
        commands.get(&name_lower).is_some_and(|cmd| !cmd.is_builtin)
    }

    pub fn tab_complete(&self, input: &str) -> Vec<String> {
        let input = input.trim_start();
        let (name, rest) = match input.split_once(' ') {
            Some((n, r)) => (n, r),
            None => (input, ""),
        };

        let name_lower = name.to_lowercase();
        let canonical = {
            let aliases = self.aliases.read().expect("lock poisoned");
            aliases.get(&name_lower).cloned().unwrap_or(name_lower)
        };

        let handler = {
            let commands = self.commands.read().expect("lock poisoned");
            commands.get(&canonical).map(|cmd| Arc::clone(&cmd.handler))
        };

        match handler {
            Some(handler) => {
                let partial_args: Vec<&str> = if rest.is_empty() {
                    vec![]
                } else if rest.ends_with(' ') {
                    let mut args: Vec<&str> = rest.split_whitespace().collect();
                    args.push("");
                    args
                } else {
                    rest.split_whitespace().collect()
                };
                handler.tab_complete(&partial_args)
            }
            None => vec![],
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
        self.register_with_plugin_id(name, aliases, description, handler, "");
    }

    fn register_with_plugin_id(
        &self,
        name: &str,
        aliases: &[&str],
        description: &str,
        handler: Box<dyn CommandHandler>,
        plugin_id: &str,
    ) {
        let name_lower = name.to_lowercase();

        {
            let commands = self.commands.read().expect("lock poisoned");
            if let Some(existing) = commands.get(&name_lower)
                && existing.is_builtin
            {
                tracing::warn!(
                    "Plugin attempted to overwrite built-in command '{name}' — ignoring"
                );
                return;
            }
        }

        self.unregister(name);

        let alias_list: Vec<String> = aliases.iter().map(|a| a.to_lowercase()).collect();

        {
            let mut alias_map = self.aliases.write().expect("lock poisoned");
            for alias in &alias_list {
                alias_map.insert(alias.clone(), name_lower.clone());
            }
        }

        {
            let mut commands = self.commands.write().expect("lock poisoned");
            commands.insert(
                name_lower,
                RegisteredCommand {
                    handler: Arc::from(handler),
                    aliases: alias_list,
                    description: description.to_string(),
                    is_builtin: false,
                    plugin_id: plugin_id.to_string(),
                },
            );
        }
    }

    fn unregister(&self, name: &str) {
        let name_lower = name.to_lowercase();

        let alias_list = {
            let commands = self.commands.read().expect("lock poisoned");
            commands
                .get(&name_lower)
                .map(|cmd| cmd.aliases.clone())
                .unwrap_or_default()
        };

        {
            let mut alias_map = self.aliases.write().expect("lock poisoned");
            for alias in &alias_list {
                alias_map.remove(alias);
            }
        }

        {
            let mut commands = self.commands.write().expect("lock poisoned");
            commands.remove(&name_lower);
        }
    }
}
