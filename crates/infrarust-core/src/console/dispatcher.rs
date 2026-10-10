use std::future::Future;
use std::pin::Pin;

use infrarust_api::command::CommandSource;

use super::ConsoleServices;
use super::output::{CommandCategory, CommandOutput, Failure, Hint};
use super::parser;
use crate::services::command_manager::DispatchOutcome;

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
        let line = line.trim_start();
        let line = line.strip_prefix('/').unwrap_or(line);
        let parsed = match parser::parse_line(line) {
            Some(p) => p,
            None => return CommandOutput::None,
        };

        let name = parsed.command.to_lowercase();

        let command = self
            .commands
            .iter()
            .find(|cmd| cmd.name() == name || cmd.aliases().iter().any(|a| *a == name));

        if let Some(cmd) = command {
            return cmd.execute(&parsed.args, services).await;
        }
        let console = CommandSource::console(services.permission_service.console_checker().await);
        match services.command_manager.dispatch(console, line).await {
            DispatchOutcome::Executed => CommandOutput::None,
            DispatchOutcome::Denied => {
                CommandOutput::error(format!("The console may not run '{name}'."))
            }
            DispatchOutcome::Unknown => {
                let plugin_names: Vec<String> = services
                    .command_manager
                    .list()
                    .into_iter()
                    .map(|info| info.spec.name)
                    .collect();
                let known = self
                    .commands
                    .iter()
                    .flat_map(|cmd| {
                        std::iter::once(cmd.name()).chain(cmd.aliases().iter().copied())
                    })
                    .chain(plugin_names.iter().map(String::as_str));
                let hint = match closest(&name, known) {
                    Some(candidate) => {
                        format!("did you mean {candidate}? type help to list commands")
                    }
                    None => "type help to list commands".to_string(),
                };
                Failure::new(format!("Unknown command '{name}'"))
                    .with_hint(Hint::Note(hint))
                    .into()
            }
        }
    }
}

fn closest<'a>(input: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (input.chars().count() / 2).clamp(1, 2);
    candidates
        .map(|candidate| (edit_distance(input, candidate), candidate))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut current = Vec::with_capacity(b.len() + 1);
        current.push(i + 1);
        for (j, right) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(left != *right);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
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
    fn closest_suggests_a_near_miss_only() {
        let names = ["kick", "kick-ip", "ban", "list"];
        assert_eq!(closest("kik", names.into_iter()), Some("kick"));
        assert_eq!(closest("lsit", names.into_iter()), Some("list"));
        assert_eq!(closest("nope", names.into_iter()), None);
        assert_eq!(closest("xyz", names.into_iter()), None);
    }

    #[test]
    fn edit_distance_counts_single_character_edits() {
        assert_eq!(edit_distance("kick", "kick"), 0);
        assert_eq!(edit_distance("kik", "kick"), 1);
        assert_eq!(edit_distance("lsit", "list"), 2);
        assert_eq!(edit_distance("", "ban"), 3);
    }

    #[test]
    fn test_register_multiple_commands() {
        let mut dispatcher = CommandDispatcher::new();
        dispatcher.register(Box::new(MockCommand));
        assert_eq!(dispatcher.commands().len(), 1);
    }
}
