use std::collections::HashSet;
use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::player::Player;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, ProtocolVersion, ServerId};

use crate::commands::actions::{broadcast, find_player, kick_player, send_player};
use crate::console::ConsoleServices;
use crate::console::commands::{args, usage};
use crate::console::dispatcher::{ConsoleCommand, ConsoleCommandSpec};
use crate::console::output::{
    Block, CommandCategory, CommandOutput, Fields, Line, OutputLine, Span, Table,
};

pub struct ListPlayersCommand;

impl ConsoleCommand for ListPlayersCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "list",
            aliases: &["players", "who", "online", "ls"],
            description: "List connected players",
            usage: "list [server]",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let filter = args.first().copied();
            let players = match filter {
                Some(server) => services
                    .player_registry
                    .get_players_on_server(&ServerId::new(server)),
                None => services.player_registry.get_all_players(),
            };
            players_output(&players, filter)
        })
    }
}

pub struct FindPlayerCommand;

impl ConsoleCommand for FindPlayerCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "find",
            aliases: &[],
            description: "Find a player by name",
            usage: "find <player>",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let Some(name) = args.first() else {
                return usage(self);
            };

            match find_player(&*services.player_registry, name) {
                Ok(player) => player_block(&*player).into(),
                Err(error) => CommandOutput::error(error.to_string()),
            }
        })
    }
}

pub struct KickCommand;

impl ConsoleCommand for KickCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "kick",
            aliases: &[],
            description: "Kick a player",
            usage: "kick <player> [reason...]",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let Some((name, reason)) = args.split_first() else {
                return usage(self);
            };
            let reason = args::rest(reason);

            match kick_player(&*services.player_registry, name, reason).await {
                Ok(kicked) => {
                    CommandOutput::Success(format!("Kicked {} ({})", kicked.player, kicked.reason))
                }
                Err(error) => CommandOutput::error(error.to_string()),
            }
        })
    }
}

pub struct KickIpCommand;

impl ConsoleCommand for KickIpCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "kick-ip",
            aliases: &["kickip"],
            description: "Kick all players from an IP",
            usage: "kick-ip <ip>",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let Some(ip_str) = args.first() else {
                return usage(self);
            };

            let ip: std::net::IpAddr = match ip_str.parse() {
                Ok(ip) => ip,
                Err(_) => return CommandOutput::error(format!("Invalid IP address: '{ip_str}'")),
            };

            let sessions = services.connection_registry.find_by_ip(&ip);
            if sessions.is_empty() {
                return CommandOutput::error(format!("No players connected from {ip}"));
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

            CommandOutput::Success(format!("Kicked {count} player(s) from {ip}"))
        })
    }
}

pub struct SendCommand;

impl ConsoleCommand for SendCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "send",
            aliases: &[],
            description: "Transfer a player to a server",
            usage: "send <player> <server>",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let [name, server, ..] = args else {
                return usage(self);
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
                Err(error) => CommandOutput::error(error.to_string()),
            }
        })
    }
}

pub struct SendAllCommand;

impl ConsoleCommand for SendAllCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "send-all",
            aliases: &["sendall"],
            description: "Transfer all players to a server",
            usage: "send-all <server>",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let Some(server) = args.first() else {
                return usage(self);
            };

            let players = services.player_registry.get_all_players();
            let target = ServerId::new(*server);
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

            let summary = format!("Sent {sent} player(s) to {server}");
            if errors > 0 {
                CommandOutput::Lines(vec![
                    OutputLine::Success(summary),
                    OutputLine::Warning(format!("{errors} transfer(s) failed")),
                ])
            } else {
                CommandOutput::Success(summary)
            }
        })
    }
}

pub struct MsgCommand;

impl ConsoleCommand for MsgCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "msg",
            aliases: &["tell", "whisper"],
            description: "Send a message to a player",
            usage: "msg <player> <message...>",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            if args.len() < 2 {
                return usage(self);
            }

            let name = args[0];
            let message = args[1..].join(" ");

            match services.player_registry.get_player(name) {
                Some(player) => {
                    if !player.is_active() {
                        return CommandOutput::error(format!(
                            "Player '{name}' is on a passive proxy path and cannot receive messages"
                        ));
                    }
                    match player.send_message(Component::text(&message)) {
                        Ok(()) => CommandOutput::Success(format!("Message sent to {name}")),
                        Err(e) => {
                            CommandOutput::error(format!("Failed to send message to {name}: {e}"))
                        }
                    }
                }
                None => CommandOutput::error(format!("Player '{name}' not found")),
            }
        })
    }
}

pub struct BroadcastCommand;

impl ConsoleCommand for BroadcastCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "broadcast",
            aliases: &["bc", "say"],
            description: "Broadcast a message to all players",
            usage: "broadcast <message...>",
            category: CommandCategory::Players,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            if args.is_empty() {
                return usage(self);
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
                Err(error) => CommandOutput::error(error.to_string()),
            }
        })
    }
}

fn players_output(players: &[Arc<dyn Player>], filter: Option<&str>) -> CommandOutput {
    if players.is_empty() {
        return CommandOutput::Note(match filter {
            Some(server) => format!("No players on {server}"),
            None => "No players online".to_string(),
        });
    }

    let mut table = Table::new(&["Player", "Server", "Mode", "Version", "Address"]);
    for player in players {
        table.row([
            Span::entity(player.profile().username.as_str()).into(),
            server_cell(&**player),
            Line::from(mode_of(&**player)),
            Line::from(version_name(player.protocol_version())),
            Span::muted(player.remote_addr().ip().to_string()).into(),
        ]);
    }

    Block::new("Players")
        .meta(list_meta(players, filter))
        .table(table)
        .into()
}

fn list_meta(players: &[Arc<dyn Player>], filter: Option<&str>) -> String {
    let online = format!("{} online", players.len());
    if let Some(server) = filter {
        return format!("{online} on {server}");
    }
    let servers: HashSet<ServerId> = players
        .iter()
        .filter_map(|player| player.current_server())
        .collect();
    match servers.len() {
        0 => online,
        1 => format!("{online} on 1 server"),
        count => format!("{online} on {count} servers"),
    }
}

fn player_block(player: &dyn Player) -> Block {
    let server = player.current_server();
    let meta = match &server {
        Some(server) => format!("online on {}", server.as_str()),
        None => "online".to_string(),
    };
    Block::new(player.profile().username.as_str())
        .meta(meta)
        .fields(
            Fields::new()
                .field("uuid", player.profile().uuid.to_string())
                .field("address", player.remote_addr().to_string())
                .field("server", server_cell(player))
                .field("mode", mode_of(player))
                .field("version", version_name(player.protocol_version())),
        )
}

fn server_cell(player: &dyn Player) -> Line {
    player
        .current_server()
        .map_or_else(|| Span::muted("-").into(), |server| server.as_str().into())
}

fn mode_of(player: &dyn Player) -> &'static str {
    if player.is_active() {
        "intercepted"
    } else {
        "passthrough"
    }
}

fn version_name(version: ProtocolVersion) -> String {
    match infrarust_protocol::version::ProtocolVersion(version.raw()).name() {
        "unknown" | "legacy" => version.raw().to_string(),
        name => name.to_string(),
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
    use crate::console::render::Renderer;
    use crate::event_bus::EventBusImpl;
    use crate::permissions::PermissionService;
    use crate::player::registry::PlayerRegistryImpl;
    use crate::player::{PlayerCommand, PlayerSession};
    use crate::plugin::PluginManagerHandle;
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
        session: Arc<PlayerSession>,
        _guard: SessionGuard,
        _commands: mpsc::Receiver<PlayerCommand>,
    }

    fn surfaces(active: bool) -> Surfaces {
        let connection_registry = Arc::new(ConnectionRegistry::new());
        let (session, commands) = PlayerSession::new_test(active);
        let guard = connection_registry.register(Arc::clone(&session));
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
        let console = ConsoleServices {
            player_registry: Arc::clone(&player_registry),
            connection_registry: Arc::clone(&connection_registry),
            ban_manager: Arc::new(BanManager::disabled(
                Arc::clone(&connection_registry),
                Arc::new(EventBusImpl::new()),
            )),
            server_manager: None,
            config_service: Arc::clone(&config_service),
            plugins: PluginManagerHandle::new(PluginManager::for_tests(Vec::new())),
            permission_service: Arc::clone(&permission_service),
            command_manager: Arc::clone(&command_manager),
            shutdown: CancellationToken::new(),
            start_time: Instant::now(),
        };
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
            session,
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
            CommandOutput::Error(failure) => failure.message,
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

    fn plain(output: &CommandOutput) -> String {
        Renderer::new(false, None).render(output)
    }

    #[tokio::test]
    async fn list_shows_each_player_with_version_and_address() {
        let world = surfaces(true);
        world.session.set_current_server(ServerId::new("lobby"));
        let output = ListPlayersCommand.execute(&[], &world.console).await;
        assert_eq!(
            plain(&output),
            "# Players - 1 online on 1 server\n\
             | PLAYER       SERVER   MODE          VERSION   ADDRESS\n\
             | TestPlayer   lobby    intercepted   1.21      127.0.0.1"
        );
    }

    #[tokio::test]
    async fn list_filtered_by_server_names_the_server() {
        let world = surfaces(false);
        world.session.set_current_server(ServerId::new("lobby"));
        let output = ListPlayersCommand.execute(&["lobby"], &world.console).await;
        assert_eq!(
            plain(&output),
            "# Players - 1 online on lobby\n\
             | PLAYER       SERVER   MODE          VERSION   ADDRESS\n\
             | TestPlayer   lobby    passthrough   1.21      127.0.0.1"
        );
        let empty = ListPlayersCommand
            .execute(&["survival"], &world.console)
            .await;
        assert_eq!(plain(&empty), "- No players on survival");
    }

    #[tokio::test]
    async fn find_describes_the_player_as_fields() {
        let world = surfaces(true);
        let uuid = world.session.profile().uuid;
        let output = FindPlayerCommand
            .execute(&["TestPlayer"], &world.console)
            .await;
        assert_eq!(
            plain(&output),
            format!(
                "# TestPlayer - online\n\
                 | uuid     {uuid}\n\
                 | address  127.0.0.1:12345\n\
                 | server   -\n\
                 | mode     intercepted\n\
                 | version  1.21"
            )
        );
    }

    #[test]
    fn unnamed_protocol_versions_show_their_number() {
        assert_eq!(version_name(ProtocolVersion::new(767)), "1.21");
        assert_eq!(version_name(ProtocolVersion::new(99999)), "99999");
        assert_eq!(
            version_name(ProtocolVersion::new(
                infrarust_protocol::version::ProtocolVersion::LEGACY.0
            )),
            infrarust_protocol::version::ProtocolVersion::LEGACY
                .0
                .to_string()
        );
    }
}
