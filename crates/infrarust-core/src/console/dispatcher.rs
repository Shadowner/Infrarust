use std::future::Future;
use std::pin::Pin;

use super::ConsoleServices;
use super::output::{CommandCategory, CommandOutput};
use super::parser;

pub trait ConsoleCommand: Send + Sync {
    fn name(&self) -> &str;

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str;

    fn usage(&self) -> &str;

    fn category(&self) -> CommandCategory;

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>>;
}

pub struct CommandInfo {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub usage: String,
    pub category: CommandCategory,
}

pub struct CommandDispatcher {
    commands: Vec<Box<dyn ConsoleCommand>>,
}

impl Default for CommandDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandDispatcher {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn register(&mut self, command: Box<dyn ConsoleCommand>) {
        self.commands.push(command);
    }

    pub fn commands(&self) -> &[Box<dyn ConsoleCommand>] {
        &self.commands
    }

    pub fn command_info(&self) -> Vec<CommandInfo> {
        self.commands
            .iter()
            .map(|cmd| CommandInfo {
                name: cmd.name().to_string(),
                aliases: cmd.aliases().iter().map(|a| a.to_string()).collect(),
                description: cmd.description().to_string(),
                usage: cmd.usage().to_string(),
                category: cmd.category(),
            })
            .collect()
    }

    pub async fn dispatch(&self, line: &str, services: &ConsoleServices) -> CommandOutput {
        let parsed = match parser::parse_line(line) {
            Some(p) => p,
            None => return CommandOutput::None,
        };

        let name = parsed.command.to_lowercase();

        let command = self
            .commands
            .iter()
            .find(|cmd| cmd.name() == name || cmd.aliases().iter().any(|a| *a == name));

        match command {
            Some(cmd) => cmd.execute(&parsed.args, services).await,
            None => CommandOutput::Error(format!(
                "Unknown command: '{name}'. Type 'help' for available commands."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    struct MockCommand;

    impl ConsoleCommand for MockCommand {
        fn name(&self) -> &str {
            "mock"
        }

        fn aliases(&self) -> &[&str] {
            &["m", "test"]
        }

        fn description(&self) -> &str {
            "A mock command"
        }

        fn usage(&self) -> &str {
            "mock [args...]"
        }

        fn category(&self) -> CommandCategory {
            CommandCategory::System
        }

        fn execute<'a>(
            &'a self,
            args: &'a [&'a str],
            _services: &'a ConsoleServices,
        ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
            Box::pin(async move {
                CommandOutput::Success(format!("mock called with {} args", args.len()))
            })
        }
    }

    #[test]
    fn test_command_info_collection() {
        let mut dispatcher = CommandDispatcher::new();
        dispatcher.register(Box::new(MockCommand));

        let infos = dispatcher.command_info();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "mock");
        assert_eq!(infos[0].aliases, vec!["m", "test"]);
        assert_eq!(infos[0].description, "A mock command");
        assert_eq!(infos[0].category, CommandCategory::System);
    }

    #[test]
    fn test_register_multiple_commands() {
        let mut dispatcher = CommandDispatcher::new();
        dispatcher.register(Box::new(MockCommand));
        assert_eq!(dispatcher.commands().len(), 1);
    }
}
