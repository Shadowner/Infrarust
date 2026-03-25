//! Server commands: servers, server, start, stop-server.

use std::future::Future;
use std::pin::Pin;

use comfy_table::Cell;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::ServerId;

use crate::console::ConsoleServices;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{CommandCategory, CommandOutput, OutputLine};

pub struct ServersCommand;

impl ConsoleCommand for ServersCommand {
    fn name(&self) -> &str {
        "servers"
    }

    fn aliases(&self) -> &[&str] {
        &["backends"]
    }

    fn description(&self) -> &str {
        "List all servers"
    }

    fn usage(&self) -> &str {
        "servers"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Servers
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let configs = services.config_service.get_all_server_configs();

            if configs.is_empty() {
                return CommandOutput::Success("No servers configured".to_string());
            }

            let managed_states: std::collections::HashMap<
                String,
                infrarust_server_manager::ServerState,
            > = services
                .server_manager
                .as_ref()
                .map(|sm| sm.get_all_managed().into_iter().collect())
                .unwrap_or_default();

            let has_managed = !managed_states.is_empty();

            let renderer = crate::console::output::OutputRenderer::new();
            let mut table = renderer.create_table();

            if has_managed {
                table.set_header(vec!["Server", "Address", "Mode", "State", "Players"]);
            } else {
                table.set_header(vec!["Server", "Address", "Mode", "Players"]);
            }

            for cfg in &configs {
                let id = cfg.id.as_str();
                let address = cfg
                    .addresses
                    .first()
                    .map(|a| format!("{}:{}", a.host, a.port))
                    .unwrap_or_else(|| "-".to_string());

                let mode = format!("{:?}", cfg.proxy_mode);
                let players = services.player_registry.online_count_on(&cfg.id);

                if has_managed {
                    let state = managed_states
                        .get(id)
                        .map(|s| format_server_state(s, services.is_tty()))
                        .unwrap_or_else(|| "-".to_string());
                    table.add_row(vec![
                        Cell::new(id),
                        Cell::new(address),
                        Cell::new(mode),
                        Cell::new(state),
                        Cell::new(players),
                    ]);
                } else {
                    table.add_row(vec![
                        Cell::new(id),
                        Cell::new(address),
                        Cell::new(mode),
                        Cell::new(players),
                    ]);
                }
            }

            CommandOutput::Table {
                table,
                footer: Some(format!(" {} server(s)", configs.len())),
            }
        })
    }
}

pub struct ServerCommand;

impl ConsoleCommand for ServerCommand {
    fn name(&self) -> &str {
        "server"
    }

    fn description(&self) -> &str {
        "Show server details"
    }

    fn usage(&self) -> &str {
        "server <id>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Servers
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let id = match args.first() {
                Some(id) => *id,
                None => return CommandOutput::Error("Usage: server <id>".to_string()),
            };

            let server_id = ServerId::new(id);
            let cfg = match services.config_service.get_server_config(&server_id) {
                Some(c) => c,
                None => return CommandOutput::Error(format!("Server '{id}' not found")),
            };

            let players = services.player_registry.online_count_on(&server_id);
            let addresses: Vec<String> = cfg
                .addresses
                .iter()
                .map(|a| format!("{}:{}", a.host, a.port))
                .collect();
            let domains = cfg.domains.join(", ");

            let mut lines = vec![
                OutputLine::Info(format!("  Server: {id}")),
                OutputLine::Info(format!("  Addresses: {}", addresses.join(", "))),
                OutputLine::Info(format!("  Domains: {domains}")),
                OutputLine::Info(format!("  Mode: {:?}", cfg.proxy_mode)),
                OutputLine::Info(format!("  Max players: {}", cfg.max_players)),
                OutputLine::Info(format!("  Players online: {players}")),
            ];

            if let Some(ref sm) = services.server_manager {
                if let Some(state) = sm.get_state(id) {
                    lines.push(OutputLine::Info(format!(
                        "  State: {}",
                        format_server_state(&state, services.is_tty())
                    )));
                }
            }

            if let Some(ref network) = cfg.network {
                lines.push(OutputLine::Info(format!("  Network: {network}")));
            }

            CommandOutput::Lines(lines)
        })
    }
}

pub struct StartServerCommand;

impl ConsoleCommand for StartServerCommand {
    fn name(&self) -> &str {
        "start"
    }

    fn description(&self) -> &str {
        "Start a server"
    }

    fn usage(&self) -> &str {
        "start <server_id>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Servers
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let sm = match services.server_manager.as_ref() {
                Some(sm) => sm,
                None => {
                    return CommandOutput::Error("Server management is not configured".to_string());
                }
            };

            let id = match args.first() {
                Some(id) => *id,
                None => return CommandOutput::Error("Usage: start <server_id>".to_string()),
            };

            tracing::info!(target: "console", server = id, "Server start requested from console");

            match sm.start_server(id).await {
                Ok(()) => CommandOutput::Success(format!("Server '{id}' started")),
                Err(e) => CommandOutput::Error(format!("Failed to start server '{id}': {e}")),
            }
        })
    }
}

pub struct StopServerCommand;

impl ConsoleCommand for StopServerCommand {
    fn name(&self) -> &str {
        "stop-server"
    }

    fn aliases(&self) -> &[&str] {
        &["stopserver"]
    }

    fn description(&self) -> &str {
        "Stop a server"
    }

    fn usage(&self) -> &str {
        "stop-server <server_id>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Servers
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let sm = match services.server_manager.as_ref() {
                Some(sm) => sm,
                None => {
                    return CommandOutput::Error("Server management is not configured".to_string());
                }
            };

            let id = match args.first() {
                Some(id) => *id,
                None => return CommandOutput::Error("Usage: stop-server <server_id>".to_string()),
            };

            tracing::info!(target: "console", server = id, "Server stop requested from console");

            match sm.stop_server(id).await {
                Ok(()) => CommandOutput::Success(format!("Server '{id}' stopped")),
                Err(e) => CommandOutput::Error(format!("Failed to stop server '{id}': {e}")),
            }
        })
    }
}

fn format_server_state(state: &infrarust_server_manager::ServerState, is_tty: bool) -> String {
    use infrarust_server_manager::ServerState;
    if is_tty {
        match state {
            ServerState::Online => format!("{} Online", console::style("\u{25CF}").green()),
            ServerState::Sleeping => format!("{} Sleep", console::style("\u{25CB}").dim()),
            ServerState::Starting => format!("{} Starting", console::style("\u{25CF}").yellow()),
            ServerState::Stopping => format!("{} Stopping", console::style("\u{25CF}").yellow()),
            ServerState::Crashed => format!("{} Crashed", console::style("\u{25CF}").red()),
            _ => format!("{} {state}", console::style("?").dim()),
        }
    } else {
        state.to_string()
    }
}
