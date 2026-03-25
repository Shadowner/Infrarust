//! Player commands: list, find, kick, kick-ip, send, send-all, msg, broadcast.

use std::future::Future;
use std::pin::Pin;

use comfy_table::Cell;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, ServerId};

use crate::console::ConsoleServices;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{CommandCategory, CommandOutput, OutputLine};

pub struct ListPlayersCommand;

impl ConsoleCommand for ListPlayersCommand {
    fn name(&self) -> &str {
        "list"
    }

    fn aliases(&self) -> &[&str] {
        &["players", "who", "online", "ls"]
    }

    fn description(&self) -> &str {
        "List connected players"
    }

    fn usage(&self) -> &str {
        "list [server]"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let players = if let Some(server) = args.first() {
                services
                    .player_registry
                    .get_players_on_server(&ServerId::new(*server))
            } else {
                services.player_registry.get_all_players()
            };

            if players.is_empty() {
                return CommandOutput::Success("No players online".to_string());
            }

            let renderer = crate::console::output::OutputRenderer::new();
            let mut table = renderer.create_table();
            table.set_header(vec!["Player", "IP", "Server", "Mode", "Protocol"]);

            for player in &players {
                let server = player
                    .current_server()
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| "-".to_string());
                let mode = if player.is_active() {
                    "active"
                } else {
                    "passthrough"
                };
                table.add_row(vec![
                    Cell::new(player.profile().username.as_str()),
                    Cell::new(player.remote_addr().ip().to_string()),
                    Cell::new(server),
                    Cell::new(mode),
                    Cell::new(player.protocol_version().to_string()),
                ]);
            }

            CommandOutput::Table {
                table,
                footer: Some(format!(" {} player(s) online", players.len())),
            }
        })
    }
}

pub struct FindPlayerCommand;

impl ConsoleCommand for FindPlayerCommand {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find a player by name"
    }

    fn usage(&self) -> &str {
        "find <player>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let name = match args.first() {
                Some(n) => *n,
                None => return CommandOutput::Error("Usage: find <player>".to_string()),
            };

            match services.player_registry.get_player(name) {
                Some(player) => {
                    let server = player
                        .current_server()
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let mode = if player.is_active() {
                        "active"
                    } else {
                        "passthrough"
                    };
                    CommandOutput::Lines(vec![
                        OutputLine::Info(format!("  Player: {}", player.profile().username)),
                        OutputLine::Info(format!("  UUID: {}", player.profile().uuid)),
                        OutputLine::Info(format!("  IP: {}", player.remote_addr())),
                        OutputLine::Info(format!("  Server: {server}")),
                        OutputLine::Info(format!("  Mode: {mode}")),
                        OutputLine::Info(format!("  Protocol: {}", player.protocol_version())),
                        OutputLine::Info(format!("  Connected: {}", player.is_connected())),
                    ])
                }
                None => CommandOutput::Error(format!("Player '{name}' not found")),
            }
        })
    }
}

pub struct KickCommand;

impl ConsoleCommand for KickCommand {
    fn name(&self) -> &str {
        "kick"
    }

    fn description(&self) -> &str {
        "Kick a player"
    }

    fn usage(&self) -> &str {
        "kick <player> [reason...]"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let name = match args.first() {
                Some(n) => *n,
                None => {
                    return CommandOutput::Error("Usage: kick <player> [reason...]".to_string());
                }
            };

            let reason = if args.len() > 1 {
                args[1..].join(" ")
            } else {
                "Kicked by administrator".to_string()
            };

            match services.player_registry.get_player(name) {
                Some(player) => {
                    let server = player
                        .current_server()
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default();
                    tracing::info!(
                        target: "console",
                        player = name,
                        reason = %reason,
                        server = %server,
                        "Player kicked from console"
                    );
                    player.disconnect(Component::text(&reason)).await;
                    CommandOutput::Success(format!("Kicked {name} (reason: {reason})"))
                }
                None => CommandOutput::Error(format!("Player '{name}' not found")),
            }
        })
    }
}

pub struct KickIpCommand;

impl ConsoleCommand for KickIpCommand {
    fn name(&self) -> &str {
        "kick-ip"
    }

    fn aliases(&self) -> &[&str] {
        &["kickip"]
    }

    fn description(&self) -> &str {
        "Kick all players from an IP"
    }

    fn usage(&self) -> &str {
        "kick-ip <ip>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let ip_str = match args.first() {
                Some(ip) => *ip,
                None => return CommandOutput::Error("Usage: kick-ip <ip>".to_string()),
            };

            let ip: std::net::IpAddr = match ip_str.parse() {
                Ok(ip) => ip,
                Err(_) => return CommandOutput::Error(format!("Invalid IP address: '{ip_str}'")),
            };

            let sessions = services.connection_registry.find_by_ip(&ip);
            if sessions.is_empty() {
                return CommandOutput::Error(format!("No players connected from {ip}"));
            }

            let count = sessions.len();
            for session in sessions {
                session.shutdown_token().cancel();
            }

            tracing::info!(
                target: "console",
                ip = %ip,
                count = count,
                "Players kicked by IP from console"
            );

            CommandOutput::Success(format!("Kicked {count} player(s) from IP {ip}"))
        })
    }
}

pub struct SendCommand;

impl ConsoleCommand for SendCommand {
    fn name(&self) -> &str {
        "send"
    }

    fn description(&self) -> &str {
        "Transfer a player to a server"
    }

    fn usage(&self) -> &str {
        "send <player> <server>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            if args.len() < 2 {
                return CommandOutput::Error("Usage: send <player> <server>".to_string());
            }

            let name = args[0];
            let server = args[1];

            match services.player_registry.get_player(name) {
                Some(player) => {
                    if !player.is_active() {
                        return CommandOutput::Error(format!(
                            "Player '{name}' is on a passive proxy path and cannot be transferred"
                        ));
                    }
                    match player.switch_server(ServerId::new(server)).await {
                        Ok(()) => {
                            tracing::info!(
                                target: "console",
                                player = name,
                                server = server,
                                "Player transferred from console"
                            );
                            CommandOutput::Success(format!("Sent {name} to {server}"))
                        }
                        Err(e) => {
                            CommandOutput::Error(format!("Failed to send {name} to {server}: {e}"))
                        }
                    }
                }
                None => CommandOutput::Error(format!("Player '{name}' not found")),
            }
        })
    }
}

pub struct SendAllCommand;

impl ConsoleCommand for SendAllCommand {
    fn name(&self) -> &str {
        "send-all"
    }

    fn aliases(&self) -> &[&str] {
        &["sendall"]
    }

    fn description(&self) -> &str {
        "Transfer all players to a server"
    }

    fn usage(&self) -> &str {
        "send-all <server>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let server = match args.first() {
                Some(s) => *s,
                None => return CommandOutput::Error("Usage: send-all <server>".to_string()),
            };

            let players = services.player_registry.get_all_players();
            let target = ServerId::new(server);
            let mut sent = 0usize;
            let mut errors = 0usize;

            for player in &players {
                if !player.is_active() {
                    continue;
                }
                match player.switch_server(target.clone()).await {
                    Ok(()) => sent += 1,
                    Err(_) => errors += 1,
                }
            }

            tracing::info!(
                target: "console",
                server = server,
                sent = sent,
                errors = errors,
                "All players transferred from console"
            );

            if errors > 0 {
                CommandOutput::Lines(vec![
                    OutputLine::Success(format!("Sent {sent} player(s) to {server}")),
                    OutputLine::Warning(format!("{errors} transfer(s) failed")),
                ])
            } else {
                CommandOutput::Success(format!("Sent {sent} player(s) to {server}"))
            }
        })
    }
}

pub struct MsgCommand;

impl ConsoleCommand for MsgCommand {
    fn name(&self) -> &str {
        "msg"
    }

    fn aliases(&self) -> &[&str] {
        &["tell", "whisper"]
    }

    fn description(&self) -> &str {
        "Send a message to a player"
    }

    fn usage(&self) -> &str {
        "msg <player> <message...>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            if args.len() < 2 {
                return CommandOutput::Error("Usage: msg <player> <message...>".to_string());
            }

            let name = args[0];
            let message = args[1..].join(" ");

            match services.player_registry.get_player(name) {
                Some(player) => {
                    if !player.is_active() {
                        return CommandOutput::Error(format!(
                            "Player '{name}' is on a passive proxy path and cannot receive messages"
                        ));
                    }
                    match player.send_message(Component::text(&message)) {
                        Ok(()) => CommandOutput::Success(format!("Message sent to {name}")),
                        Err(e) => {
                            CommandOutput::Error(format!("Failed to send message to {name}: {e}"))
                        }
                    }
                }
                None => CommandOutput::Error(format!("Player '{name}' not found")),
            }
        })
    }
}

pub struct BroadcastCommand;

impl ConsoleCommand for BroadcastCommand {
    fn name(&self) -> &str {
        "broadcast"
    }

    fn aliases(&self) -> &[&str] {
        &["bc", "say"]
    }

    fn description(&self) -> &str {
        "Broadcast a message to all players"
    }

    fn usage(&self) -> &str {
        "broadcast <message...>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            if args.is_empty() {
                return CommandOutput::Error("Usage: broadcast <message...>".to_string());
            }

            let message = args.join(" ");
            let component = Component::text(&message);
            let players = services.player_registry.get_all_players();
            let mut sent = 0usize;

            for player in &players {
                if !player.is_active() {
                    continue;
                }
                if player.send_message(component.clone()).is_ok() {
                    sent += 1;
                }
            }

            tracing::info!(
                target: "console",
                message = %message,
                recipients = sent,
                "Broadcast sent from console"
            );

            CommandOutput::Success(format!("Broadcast sent to {sent} player(s)"))
        })
    }
}
