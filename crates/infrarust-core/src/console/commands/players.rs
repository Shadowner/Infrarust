//! Player commands: list, find, kick, kick-ip, send, send-all, msg, broadcast.

use std::future::Future;
use std::pin::Pin;

use comfy_table::Cell;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, ServerId};

use crate::commands::actions::{broadcast, find_player, kick_player, send_player};
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
            let Some(name) = args.first() else {
                return CommandOutput::Error("Usage: find <player>".to_string());
            };

            match find_player(&*services.player_registry, name) {
                Ok(player) => {
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
                Err(error) => CommandOutput::Error(error.to_string()),
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
            let Some((name, reason)) = args.split_first() else {
                return CommandOutput::Error("Usage: kick <player> [reason...]".to_string());
            };
            let reason = (!reason.is_empty()).then(|| reason.join(" "));

            match kick_player(&*services.player_registry, name, reason).await {
                Ok(kicked) => CommandOutput::Success(format!(
                    "Kicked {} (reason: {})",
                    kicked.player, kicked.reason
                )),
                Err(error) => CommandOutput::Error(error.to_string()),
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
            let [name, server, ..] = args else {
                return CommandOutput::Error("Usage: send <player> <server>".to_string());
            };

            match send_player(
                &*services.player_registry,
                &*services.config_service,
                name,
                server,
            )
            .await
            {
                Ok(sent) => {
                    CommandOutput::Success(format!("Sent {} to {}", sent.player, sent.server))
                }
                Err(error) => CommandOutput::Error(error.to_string()),
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

            match broadcast(
                &*services.player_registry,
                &*services.config_service,
                None,
                &args.join(" "),
            ) {
                Ok(sent) => CommandOutput::Success(format!(
                    "Broadcast sent to {} player(s)",
                    sent.recipients
                )),
                Err(error) => CommandOutput::Error(error.to_string()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    use infrarust_api::command::CommandContext;
    use infrarust_api::test_util::{MockPlayer, player_source};
    use infrarust_config::PermissionsConfig;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::ban::manager::BanManager;
    use crate::commands::{CommandServices, SubcommandHandler, subcommands};
    use crate::event_bus::EventBusImpl;
    use crate::permissions::PermissionService;
    use crate::player::registry::PlayerRegistryImpl;
    use crate::player::{PlayerCommand, PlayerSession};
    use crate::plugin::PluginRegistryImpl;
    use crate::plugin::manager::PluginManager;
    use crate::provider::ProviderId;
    use crate::registry::{ConnectionRegistry, SessionGuard};
    use crate::routing::DomainRouter;
    use crate::services::command_manager::CommandManagerImpl;
    use crate::services::config_service::ConfigServiceImpl;

    struct Surfaces {
        console: ConsoleServices,
        ir: CommandServices,
        _guard: SessionGuard,
        _commands: mpsc::Receiver<PlayerCommand>,
    }

    fn surfaces(active: bool) -> Surfaces {
        let connection_registry = Arc::new(ConnectionRegistry::new());
        let (session, commands) = PlayerSession::new_test(active);
        let guard = connection_registry.register(session);
        let player_registry = Arc::new(PlayerRegistryImpl::new(Arc::clone(&connection_registry)));
        let domain_router = Arc::new(DomainRouter::new());
        domain_router.add(
            ProviderId::file("lobby"),
            toml::from_str(
                "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"127.0.0.1:25566\"]\n",
            )
            .unwrap(),
        );
        let config_service = Arc::new(ConfigServiceImpl::new(
            domain_router,
            PathBuf::from("infrarust.toml"),
            Arc::new(toml::from_str("").unwrap()),
        ));
        let permission_service =
            Arc::new(PermissionService::new_sync(&PermissionsConfig::default()));
        let command_manager = Arc::new(CommandManagerImpl::new());
        let console = ConsoleServices::new(
            Arc::clone(&player_registry),
            Arc::clone(&connection_registry),
            Arc::new(BanManager::disabled(
                Arc::clone(&connection_registry),
                Arc::new(EventBusImpl::new()),
            )),
            None,
            Arc::clone(&config_service),
            Arc::new(tokio::sync::RwLock::new(PluginManager::new(Vec::new()))),
            Arc::clone(&permission_service),
            Arc::clone(&command_manager),
            CancellationToken::new(),
            Instant::now(),
        );
        let ir = CommandServices {
            player_registry,
            config_service,
            server_manager: None,
            plugin_registry: Arc::new(PluginRegistryImpl::new()),
            command_manager,
            permission_service,
            start_time: Instant::now(),
        };
        Surfaces {
            console,
            ir,
            _guard: guard,
            _commands: commands,
        }
    }

    async fn ir_reply(
        services: &CommandServices,
        sub: &dyn SubcommandHandler,
        args: &[&str],
    ) -> String {
        let admin = MockPlayer::new(9, "Admin")
            .with_all_permissions()
            .into_arc();
        let ctx = CommandContext::new(player_source(&admin), "ir", args.join(" "));
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        sub.execute(&ctx, &args, services).await;
        admin.sent_text()
    }

    fn error_text(output: CommandOutput) -> String {
        match output {
            CommandOutput::Error(text) => text,
            CommandOutput::Success(text) => panic!("expected an error, got success: {text}"),
            _ => panic!("expected an error line"),
        }
    }

    #[tokio::test]
    async fn send_rejects_an_unknown_server_the_same_way_on_both_surfaces() {
        let world = surfaces(true);
        let console = error_text(
            SendCommand
                .execute(&["TestPlayer", "nowhere"], &world.console)
                .await,
        );
        let ir = ir_reply(
            &world.ir,
            &subcommands::send::SendSubcommand,
            &["TestPlayer", "nowhere"],
        )
        .await;
        assert!(console.contains("nowhere"), "{console}");
        assert!(ir.contains(&console), "console: {console}\n/ir: {ir}");
    }

    #[tokio::test]
    async fn send_rejects_a_passive_player_the_same_way_on_both_surfaces() {
        let world = surfaces(false);
        let console = error_text(
            SendCommand
                .execute(&["TestPlayer", "lobby"], &world.console)
                .await,
        );
        let ir = ir_reply(
            &world.ir,
            &subcommands::send::SendSubcommand,
            &["TestPlayer", "lobby"],
        )
        .await;
        assert!(console.contains("TestPlayer"), "{console}");
        assert!(ir.contains(&console), "console: {console}\n/ir: {ir}");
    }
}
