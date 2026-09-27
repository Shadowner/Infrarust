use std::future::Future;
use std::io::IsTerminal;
use std::pin::Pin;

use infrarust_api::command::CommandInfo as PluginCommandInfo;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::console::ConsoleServices;
use crate::console::commands::servers::StateCounts;
use crate::console::dispatcher::{CommandInfo, ConsoleCommand};
use crate::console::output::{
    Block, CommandCategory, CommandOutput, Failure, Fields, Hint, Line, Span, Table,
};
use crate::console::parser::format_duration_short;
use crate::console::render::usage_line;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn product_title() -> String {
    format!("Infrarust {VERSION}")
}

pub(crate) fn help_block(commands: &[CommandInfo], plugin_commands: &[PluginCommandInfo]) -> Block {
    let plugin_rows: Vec<(String, &PluginCommandInfo)> = plugin_commands
        .iter()
        .filter_map(|info| info.namespaced().map(|namespaced| (namespaced, info)))
        .collect();

    let mut meta = format!("{} built-in", commands.len());
    if !plugin_rows.is_empty() {
        meta.push_str(&format!(", {} from plugins", plugin_rows.len()));
    }
    meta.push_str(", help <command> for details");

    let mut block = Block::new("Commands").meta(meta);
    let mut first = true;
    for category in CommandCategory::ALL {
        let mut table = Table::bare();
        for info in commands.iter().filter(|info| info.category == category) {
            table.row([
                usage_line(&info.usage),
                Span::muted(info.description.as_str()).into(),
            ]);
        }
        if table.is_empty() {
            continue;
        }
        if !first {
            block = block.gap();
        }
        first = false;
        block = block.heading(category.display_name()).table(table);
    }

    if !plugin_rows.is_empty() {
        let mut table = Table::bare();
        for (namespaced, info) in plugin_rows {
            table.row([
                Line::from(vec![
                    Span::plain(info.spec.name.as_str()),
                    Span::muted(format!(" ({namespaced})")),
                ]),
                Span::muted(info.spec.description.as_str()).into(),
            ]);
        }
        if !first {
            block = block.gap();
        }
        block = block.heading("Plugin commands").table(table);
    }

    block
}

pub(crate) fn command_block(info: &CommandInfo) -> Block {
    let mut fields = Fields::new().field("usage", usage_line(&info.usage));
    if !info.aliases.is_empty() {
        fields.push("aliases", info.aliases.join(", "));
    }
    fields.push("category", info.category.display_name());
    Block::new(info.name.as_str())
        .meta(info.description.as_str())
        .fields(fields)
}

pub(crate) fn unknown_command(name: &str) -> Failure {
    Failure::new(format!("Unknown command '{name}'"))
        .with_hint(Hint::Note("type help to list commands".to_string()))
}

pub(crate) fn minecraft_range(versions: &[ProtocolVersion]) -> Option<String> {
    let mut names = versions
        .iter()
        .map(|version| version.name())
        .filter(|name| !matches!(*name, "legacy" | "unknown"));
    let oldest = names.next()?;
    let newest = names.next_back().unwrap_or(oldest);
    Some(format!("{oldest} -> {newest}"))
}

pub(crate) fn version_block() -> Block {
    let mut fields = Fields::new();
    if let Some(range) = minecraft_range(ProtocolVersion::SUPPORTED) {
        fields.push("minecraft", range);
    }
    fields.push(
        "platform",
        format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
    );
    Block::new(product_title()).fields(fields)
}

pub(crate) struct StatusReport {
    pub uptime: String,
    pub players: usize,
    pub connections: usize,
    pub servers: usize,
    pub managed: Option<StateCounts>,
    pub plugins: usize,
}

pub(crate) fn status_block(report: &StatusReport) -> Block {
    let players_text = format!("{} online", report.players);
    let players = if report.players > 0 {
        Span::ok(players_text)
    } else {
        Span::plain(players_text)
    };

    let mut servers = Line::new().push(format!("{} configured", report.servers));
    if let Some(counts) = report.managed {
        for (mark, label) in counts.labels() {
            servers = servers.push("  ").push(Span::marked(mark, label));
        }
    }

    Block::new(product_title())
        .meta(format!("up {}", report.uptime))
        .fields(
            Fields::new()
                .field("players", players)
                .field("connections", report.connections.to_string())
                .field("servers", servers)
                .field("plugins", format!("{} loaded", report.plugins)),
        )
}

pub struct HelpCommand {
    commands: Vec<CommandInfo>,
}

impl HelpCommand {
    pub fn from_commands(mut commands: Vec<CommandInfo>) -> Self {
        let help = Self {
            commands: Vec::new(),
        };
        commands.push(CommandInfo {
            name: help.name().to_string(),
            aliases: help.aliases().iter().map(ToString::to_string).collect(),
            description: help.description().to_string(),
            usage: help.usage().to_string(),
            category: help.category(),
        });
        Self { commands }
    }
}

impl ConsoleCommand for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }

    fn aliases(&self) -> &[&str] {
        &["?"]
    }

    fn description(&self) -> &str {
        "Show this help"
    }

    fn usage(&self) -> &str {
        "help [command]"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::System
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let Some(name) = args.first() else {
                return help_block(&self.commands, &services.command_manager.list()).into();
            };
            let lower = name.to_lowercase();
            self.commands
                .iter()
                .find(|info| info.name == lower || info.aliases.iter().any(|a| a == &lower))
                .map_or_else(
                    || unknown_command(&lower).into(),
                    |info| command_block(info).into(),
                )
        })
    }
}

pub struct VersionCommand;

impl ConsoleCommand for VersionCommand {
    fn name(&self) -> &str {
        "version"
    }

    fn aliases(&self) -> &[&str] {
        &["ver"]
    }

    fn description(&self) -> &str {
        "Show version info"
    }

    fn usage(&self) -> &str {
        "version"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::System
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        _services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move { version_block().into() })
    }
}

pub struct StatusCommand;

impl ConsoleCommand for StatusCommand {
    fn name(&self) -> &str {
        "status"
    }

    fn aliases(&self) -> &[&str] {
        &["info"]
    }

    fn description(&self) -> &str {
        "Proxy overview"
    }

    fn usage(&self) -> &str {
        "status"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::System
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let report = StatusReport {
                uptime: format_duration_short(services.start_time.elapsed()),
                players: services.player_registry.online_count(),
                connections: services.connection_registry.count(),
                servers: services.config_service.get_all_server_configs().len(),
                managed: services.server_manager.as_ref().map(|manager| {
                    StateCounts::tally(manager.get_all_managed().iter().map(|(_, state)| state))
                }),
                plugins: services.plugin_manager.read().await.list_plugins().len(),
            };
            status_block(&report).into()
        })
    }
}

pub struct StopCommand;

impl ConsoleCommand for StopCommand {
    fn name(&self) -> &str {
        "stop"
    }

    fn aliases(&self) -> &[&str] {
        &["shutdown", "exit", "quit"]
    }

    fn description(&self) -> &str {
        "Shutdown the proxy"
    }

    fn usage(&self) -> &str {
        "stop"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::System
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!(target: "console", "Proxy shutdown initiated from console");
            services.shutdown.cancel();
            CommandOutput::Success("Shutting down".to_string())
        })
    }
}

pub struct ClearCommand;

impl ConsoleCommand for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }

    fn aliases(&self) -> &[&str] {
        &["cls"]
    }

    fn description(&self) -> &str {
        "Clear the screen"
    }

    fn usage(&self) -> &str {
        "clear"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::System
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        _services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            if std::io::stdout().is_terminal() {
                print!("\x1B[2J\x1B[H");
            }
            CommandOutput::None
        })
    }
}

pub struct GcCommand;

impl ConsoleCommand for GcCommand {
    fn name(&self) -> &str {
        "gc"
    }

    fn description(&self) -> &str {
        "Run garbage collection"
    }

    fn usage(&self) -> &str {
        "gc"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::System
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let Some(builtin) = services.ban_manager.builtin_provider() else {
                return CommandOutput::Success("Garbage collected".to_string());
            };
            match builtin.storage().get_all_active().await {
                Ok(bans) => CommandOutput::Success(format!(
                    "Garbage collected, {} active ban{} left",
                    bans.len(),
                    if bans.len() == 1 { "" } else { "s" }
                )),
                Err(e) => CommandOutput::error(format!("GC failed: {e}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use infrarust_api::command::CommandSpec;

    use super::*;
    use crate::console::render::Renderer;

    fn render(output: impl Into<CommandOutput>) -> String {
        Renderer::new(false, None).render(&output.into())
    }

    fn info(
        name: &str,
        aliases: &[&str],
        description: &str,
        usage: &str,
        category: CommandCategory,
    ) -> CommandInfo {
        CommandInfo {
            name: name.to_string(),
            aliases: aliases.iter().map(ToString::to_string).collect(),
            description: description.to_string(),
            usage: usage.to_string(),
            category,
        }
    }

    fn builtins() -> Vec<CommandInfo> {
        vec![
            info(
                "list",
                &["ls"],
                "List online players",
                "list [server]",
                CommandCategory::Players,
            ),
            info(
                "kick",
                &[],
                "Kick a player",
                "kick <player> [reason...]",
                CommandCategory::Players,
            ),
            info(
                "ban",
                &[],
                "Ban a player",
                "ban <player> [duration] [reason...]",
                CommandCategory::Bans,
            ),
            info(
                "status",
                &["info"],
                "Proxy overview",
                "status",
                CommandCategory::System,
            ),
        ]
    }

    #[test]
    fn help_groups_commands_by_category() {
        assert_eq!(
            render(help_block(&builtins(), &[])),
            "# Commands - 4 built-in, help <command> for details\n\
             | PLAYERS\n\
             | list [server]               List online players\n\
             | kick <player> [reason...]   Kick a player\n\
             |\n\
             | BANS\n\
             | ban <player> [duration] [reason...]   Ban a player\n\
             |\n\
             | SYSTEM\n\
             | status   Proxy overview"
        );
    }

    #[test]
    fn help_lists_namespaced_plugin_commands() {
        let plugin = vec![
            PluginCommandInfo::new(
                CommandSpec::new("lobby").description("Go to the lobby"),
                Some("hub".to_string()),
            ),
            PluginCommandInfo::new(CommandSpec::new("core"), None),
        ];
        let rendered = render(help_block(&builtins()[3..], &plugin));
        assert_eq!(
            rendered,
            "# Commands - 1 built-in, 1 from plugins, help <command> for details\n\
             | SYSTEM\n\
             | status   Proxy overview\n\
             |\n\
             | PLUGIN COMMANDS\n\
             | lobby (hub:lobby)   Go to the lobby"
        );
    }

    #[test]
    fn help_for_one_command_shows_its_usage_and_aliases() {
        assert_eq!(
            render(command_block(&builtins()[2])),
            "# ban - Ban a player\n\
             | usage     ban <player> [duration] [reason...]\n\
             | category  Bans"
        );
        assert_eq!(
            render(command_block(&builtins()[0])),
            "# list - List online players\n\
             | usage     list [server]\n\
             | aliases   ls\n\
             | category  Players"
        );
    }

    #[test]
    fn help_for_an_unknown_command_points_to_help() {
        assert_eq!(
            render(unknown_command("nope")),
            "error: Unknown command 'nope'\n  type help to list commands"
        );
    }

    #[test]
    fn help_includes_itself() {
        let help = HelpCommand::from_commands(builtins());
        let own = help.commands.last().unwrap();
        assert_eq!(own.name, "help");
        assert_eq!(own.aliases, ["?"]);
    }

    #[test]
    fn the_minecraft_range_skips_legacy_and_unknown() {
        assert_eq!(
            minecraft_range(&[
                ProtocolVersion::LEGACY,
                ProtocolVersion::V1_8,
                ProtocolVersion::V1_21,
                ProtocolVersion::UNKNOWN,
            ])
            .as_deref(),
            Some("1.8 -> 1.21")
        );
        assert_eq!(minecraft_range(&[ProtocolVersion::LEGACY]), None);
        assert_eq!(
            minecraft_range(ProtocolVersion::SUPPORTED).as_deref(),
            Some("1.7.2 -> 26.3")
        );
    }

    #[test]
    fn version_names_the_build_and_the_platform() {
        assert_eq!(
            render(version_block()),
            format!(
                "# Infrarust {VERSION}\n\
                 | minecraft  1.7.2 -> 26.3\n\
                 | platform   {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        );
    }

    fn report(managed: Option<StateCounts>) -> StatusReport {
        StatusReport {
            uptime: "2h 5m".to_string(),
            players: 12,
            connections: 14,
            servers: 3,
            managed,
            plugins: 2,
        }
    }

    #[test]
    fn status_summarises_the_proxy() {
        assert_eq!(
            render(status_block(&report(Some(StateCounts {
                online: 2,
                sleeping: 1,
                crashed: 0,
            })))),
            format!(
                "# Infrarust {VERSION} - up 2h 5m\n\
                 | players      12 online\n\
                 | connections  14\n\
                 | servers      3 configured  2 online  1 sleeping\n\
                 | plugins      2 loaded"
            )
        );
    }

    #[test]
    fn status_marks_managed_server_counts() {
        let block = status_block(&report(Some(StateCounts {
            online: 1,
            sleeping: 0,
            crashed: 2,
        })));
        let crate::console::output::Node::Fields(fields) = &block.body[0] else {
            panic!("status is a list of fields");
        };
        let (_, servers) = &fields.0[2];
        assert_eq!(
            servers.0[2],
            Span::marked(crate::terminal::Mark::Up, "1 online")
        );
        assert_eq!(
            servers.0[6],
            Span::marked(crate::terminal::Mark::Down, "2 crashed")
        );
        let (_, players) = &fields.0[0];
        assert_eq!(players.0[0], Span::ok("12 online"));
    }

    #[test]
    fn status_without_a_manager_only_counts_servers() {
        let mut idle = report(None);
        idle.players = 0;
        assert_eq!(
            render(status_block(&idle)),
            format!(
                "# Infrarust {VERSION} - up 2h 5m\n\
                 | players      0 online\n\
                 | connections  14\n\
                 | servers      3 configured\n\
                 | plugins      2 loaded"
            )
        );
    }
}
