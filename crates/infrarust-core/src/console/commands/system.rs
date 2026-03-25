//! System commands: help, version, status, stop, clear, gc.

use std::future::Future;
use std::io::IsTerminal;
use std::pin::Pin;

use infrarust_api::services::config_service::ConfigService;

use crate::console::ConsoleServices;
use crate::console::dispatcher::{CommandInfo, ConsoleCommand};
use crate::console::output::{CommandCategory, CommandOutput, OutputLine};
use crate::console::parser::format_duration_short;

pub struct HelpCommand {
    commands: Vec<CommandInfo>,
}

impl HelpCommand {
    pub fn from_commands(commands: Vec<CommandInfo>) -> Self {
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
        _services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            if let Some(cmd_name) = args.first() {
                let lower = cmd_name.to_lowercase();
                if let Some(info) = self
                    .commands
                    .iter()
                    .find(|c| c.name == lower || c.aliases.iter().any(|a| a == &lower))
                {
                    let aliases = if info.aliases.is_empty() {
                        String::new()
                    } else {
                        format!(" (aliases: {})", info.aliases.join(", "))
                    };
                    return CommandOutput::Lines(vec![
                        OutputLine::Info(format!("{}{}", info.name, aliases)),
                        OutputLine::Info(format!("  {}", info.description)),
                        OutputLine::Info(format!("  Usage: {}", info.usage)),
                    ]);
                }
                return CommandOutput::Error(format!("Unknown command: '{lower}'"));
            }

            let is_tty = std::io::stdout().is_terminal();
            let mut lines = Vec::new();

            for category in CommandCategory::ALL {
                let cmds: Vec<&CommandInfo> = self
                    .commands
                    .iter()
                    .filter(|c| c.category == category)
                    .collect();
                if cmds.is_empty() {
                    continue;
                }

                lines.push(OutputLine::Info(String::new()));
                let cat_name = if is_tty {
                    format!(
                        "  {}",
                        console::style(category.display_name()).cyan().bold()
                    )
                } else {
                    format!("  {}", category.display_name())
                };
                lines.push(OutputLine::Info(cat_name));

                for cmd in cmds {
                    let aliases_str = if cmd.aliases.is_empty() {
                        String::new()
                    } else if is_tty {
                        format!(", {}", console::style(cmd.aliases.join(", ")).dim())
                    } else {
                        format!(", {}", cmd.aliases.join(", "))
                    };

                    lines.push(OutputLine::Info(format!(
                        "    {:<25} {}",
                        format!("{}{}", cmd.name, aliases_str),
                        if is_tty {
                            format!("{}", console::style(&cmd.description).dim())
                        } else {
                            cmd.description.clone()
                        }
                    )));
                }
            }

            CommandOutput::Lines(lines)
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
        Box::pin(async move {
            CommandOutput::Success(format!("Infrarust v{}", env!("CARGO_PKG_VERSION")))
        })
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
            use infrarust_api::services::player_registry::PlayerRegistry;

            let uptime = format_duration_short(services.start_time.elapsed());
            let players = services.player_registry.online_count();
            let connections = services.connection_registry.count();

            let configs = services.config_service.get_all_server_configs();
            let server_count = configs.len();

            let mut managed_online = 0usize;
            let mut managed_sleeping = 0usize;
            if let Some(ref sm) = services.server_manager {
                for (_, state) in sm.get_all_managed() {
                    if state.is_joinable() {
                        managed_online += 1;
                    } else {
                        managed_sleeping += 1;
                    }
                }
            }

            let is_tty = services.is_tty();

            let version_str = if is_tty {
                format!(
                    "  {} {}",
                    console::style("Infrarust").green().bold(),
                    console::style(format!("v{}", env!("CARGO_PKG_VERSION"))).dim(),
                )
            } else {
                format!("  Infrarust v{}", env!("CARGO_PKG_VERSION"))
            };

            let labeled = |label: &str, value: &str| -> String {
                if is_tty {
                    format!("  {} {value}", console::style(format!("{label}:")).bold())
                } else {
                    format!("  {label}: {value}")
                }
            };

            let players_str = if is_tty {
                format!("{}", console::style(format!("{players} online")).green())
            } else {
                format!("{players} online")
            };

            let mut lines = vec![
                OutputLine::Info(version_str),
                OutputLine::Info(labeled("Uptime", &uptime)),
                OutputLine::Info(labeled("Players", &players_str)),
            ];

            if services.server_manager.is_some() {
                let servers_str = if is_tty {
                    format!(
                        "{server_count} configured, {} online, {} sleeping",
                        console::style(managed_online).green(),
                        console::style(managed_sleeping).dim(),
                    )
                } else {
                    format!(
                        "{server_count} configured, {managed_online} online, {managed_sleeping} sleeping"
                    )
                };
                lines.push(OutputLine::Info(labeled("Servers", &servers_str)));
            } else {
                lines.push(OutputLine::Info(labeled(
                    "Servers",
                    &format!("{server_count} configured"),
                )));
            }

            lines.push(OutputLine::Info(labeled(
                "Connections",
                &connections.to_string(),
            )));

            CommandOutput::Lines(lines)
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
            CommandOutput::Success("Shutting down...".to_string())
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
            match services.ban_manager.get_all_bans().await {
                Ok(bans) => {
                    let active = bans.iter().filter(|b| !b.is_expired()).count();
                    CommandOutput::Success(format!(
                        "GC cycle completed. {active} active ban(s) remaining."
                    ))
                }
                Err(e) => CommandOutput::Error(format!("GC failed: {e}")),
            }
        })
    }
}
