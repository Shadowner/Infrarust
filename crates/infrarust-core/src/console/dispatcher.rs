use infrarust_api::command::CommandSource;
use infrarust_api::event::BoxFuture;

use super::ConsoleServices;
use super::output::{CommandCategory, CommandOutput, Failure, Hint};
use super::parser;
use crate::services::command_manager::DispatchOutcome;

#[derive(Debug)]
pub struct ConsoleCommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub usage: &'static str,
    pub category: CommandCategory,
}

impl ConsoleCommandSpec {
    pub fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.aliases.contains(&name)
    }

    fn names<'a>(&self) -> impl Iterator<Item = &'a str> {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }
}

pub trait ConsoleCommand: Send + Sync {
    fn spec(&self) -> &'static ConsoleCommandSpec;

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput>;
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

    pub fn specs(&self) -> Vec<&'static ConsoleCommandSpec> {
        self.commands.iter().map(|cmd| cmd.spec()).collect()
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
            .find(|cmd| cmd.spec().answers_to(&name));

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
                    .flat_map(|cmd| cmd.spec().names())
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
        fn spec(&self) -> &'static ConsoleCommandSpec {
            &ConsoleCommandSpec {
                name: "mock",
                aliases: &["m", "test"],
                description: "A mock command",
                usage: "mock [args...]",
                category: CommandCategory::System,
            }
        }

        fn execute<'a>(
            &'a self,
            args: &'a [&'a str],
            _services: &'a ConsoleServices,
        ) -> BoxFuture<'a, CommandOutput> {
            Box::pin(async move {
                CommandOutput::Success(format!("mock called with {} args", args.len()))
            })
        }
    }

    #[test]
    fn a_spec_answers_to_its_name_and_aliases() {
        let spec = MockCommand.spec();
        assert!(spec.answers_to("mock"));
        assert!(spec.answers_to("test"));
        assert!(!spec.answers_to("moc"));
        assert_eq!(spec.names().collect::<Vec<_>>(), ["mock", "m", "test"]);
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
