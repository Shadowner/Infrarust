use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use infrarust_api::services::config_service::{ConfigService, ProxyMode, ServerConfig};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{ServerAddress, ServerId};
use infrarust_server_manager::ServerState;

use crate::console::ConsoleServices;
use crate::console::commands::usage;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{Block, CommandCategory, CommandOutput, Fields, Line, Span, Table};
use crate::terminal::Mark;

pub(crate) fn state_span(state: ServerState) -> Span {
    match state {
        ServerState::Online => Span::marked(Mark::Up, "online"),
        ServerState::Sleeping => Span::marked(Mark::Idle, "sleeping"),
        ServerState::Starting => Span::marked(Mark::Busy, "starting"),
        ServerState::Stopping => Span::marked(Mark::Busy, "stopping"),
        ServerState::Crashed => Span::marked(Mark::Down, "crashed"),
        _ => Span::muted("unknown"),
    }
}

pub(crate) const fn mode_name(mode: ProxyMode) -> &'static str {
    match mode {
        ProxyMode::Passthrough => "passthrough",
        ProxyMode::ZeroCopy => "zero_copy",
        ProxyMode::ClientOnly => "client_only",
        ProxyMode::Offline => "offline",
        ProxyMode::ServerOnly => "server_only",
        _ => "unknown",
    }
}

pub(crate) fn join_addresses(addresses: &[ServerAddress]) -> String {
    if addresses.is_empty() {
        return "-".to_string();
    }
    addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StateCounts {
    pub online: usize,
    pub sleeping: usize,
    pub crashed: usize,
}

impl StateCounts {
    pub fn tally<'a>(states: impl IntoIterator<Item = &'a ServerState>) -> Self {
        states
            .into_iter()
            .fold(Self::default(), |mut counts, state| {
                match state {
                    ServerState::Online => counts.online += 1,
                    ServerState::Sleeping => counts.sleeping += 1,
                    ServerState::Crashed => counts.crashed += 1,
                    _ => {}
                }
                counts
            })
    }

    pub fn labels(self) -> Vec<(Mark, String)> {
        let mut labels = vec![
            (Mark::Up, format!("{} online", self.online)),
            (Mark::Idle, format!("{} sleeping", self.sleeping)),
        ];
        if self.crashed > 0 {
            labels.push((Mark::Down, format!("{} crashed", self.crashed)));
        }
        labels
    }
}

pub(crate) struct ServerView {
    pub config: ServerConfig,
    pub players: usize,
}

fn players_text(online: usize, max: u32, separator: &str) -> String {
    if max > 0 {
        format!("{online}{separator}{max}")
    } else {
        online.to_string()
    }
}

fn address_cell(addresses: &[ServerAddress]) -> Line {
    match addresses.split_first() {
        None => Span::muted("-").into(),
        Some((first, [])) => Span::muted(first.to_string()).into(),
        Some((first, rest)) => Span::muted(format!("{first} +{}", rest.len())).into(),
    }
}

pub(crate) fn servers_output(
    servers: &[ServerView],
    states: Option<&HashMap<String, ServerState>>,
) -> CommandOutput {
    if servers.is_empty() {
        return CommandOutput::Note("No servers configured".to_string());
    }

    let mut meta = format!("{} configured", servers.len());
    if let Some(states) = states {
        let counts = StateCounts::tally(states.values());
        for (_, label) in counts.labels() {
            meta.push_str(", ");
            meta.push_str(&label);
        }
    }

    let managed = states.filter(|states| !states.is_empty());
    let mut table = if managed.is_some() {
        Table::new(&["Server", "State", "Players", "Mode", "Address"])
    } else {
        Table::new(&["Server", "Players", "Mode", "Address"])
    };

    for server in servers {
        let config = &server.config;
        let mut cells: Vec<Line> = vec![Span::entity(config.id.as_str()).into()];
        if let Some(states) = managed {
            cells.push(
                states
                    .get(config.id.as_str())
                    .map_or_else(|| Span::muted("-"), |state| state_span(*state))
                    .into(),
            );
        }
        cells.push(players_text(server.players, config.max_players, "/").into());
        cells.push(mode_name(config.proxy_mode).into());
        cells.push(address_cell(&config.addresses));
        table.row(cells);
    }

    Block::new("Servers").meta(meta).table(table).into()
}

pub(crate) fn server_block(server: &ServerView, state: Option<ServerState>) -> Block {
    let config = &server.config;
    let domains = if config.domains.is_empty() {
        "-".to_string()
    } else {
        config.domains.join(", ")
    };

    let mut fields = Fields::new()
        .field("addresses", join_addresses(&config.addresses))
        .field("domains", domains)
        .field("mode", mode_name(config.proxy_mode))
        .field(
            "players",
            players_text(server.players, config.max_players, " / "),
        );
    if let Some(network) = &config.network {
        fields.push("network", network.as_str());
    }

    let block = Block::new(config.id.as_str());
    match state {
        Some(state) => block.meta(state_span(state)),
        None => block,
    }
    .fields(fields)
}

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
            let servers: Vec<ServerView> = services
                .config_service
                .get_all_server_configs()
                .into_iter()
                .map(|config| ServerView {
                    players: services.player_registry.online_count_on(&config.id),
                    config,
                })
                .collect();
            let states: Option<HashMap<String, ServerState>> = services
                .server_manager
                .as_ref()
                .map(|manager| manager.get_all_managed().into_iter().collect());
            servers_output(&servers, states.as_ref())
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
            let Some(id) = args.first() else {
                return usage(self);
            };

            let server_id = ServerId::new(*id);
            let Some(config) = services.config_service.get_server_config(&server_id) else {
                return CommandOutput::error(format!("Server '{id}' not found"));
            };

            let server = ServerView {
                players: services.player_registry.online_count_on(&server_id),
                config,
            };
            let state = services
                .server_manager
                .as_ref()
                .and_then(|manager| manager.get_state(id));
            server_block(&server, state).into()
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
            let Some(manager) = services.server_manager.as_ref() else {
                return CommandOutput::error("Server management is not configured");
            };

            let Some(id) = args.first() else {
                return usage(self);
            };

            tracing::info!(target: "console", server = id, "Server start requested from console");

            match manager.start_server(id).await {
                Ok(()) => CommandOutput::Success(format!("Started {id}")),
                Err(e) => CommandOutput::error(format!("Failed to start server '{id}': {e}")),
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
            let Some(manager) = services.server_manager.as_ref() else {
                return CommandOutput::error("Server management is not configured");
            };

            let Some(id) = args.first() else {
                return usage(self);
            };

            tracing::info!(target: "console", server = id, "Server stop requested from console");

            match manager.stop_server(id).await {
                Ok(()) => CommandOutput::Success(format!("Stopped {id}")),
                Err(e) => CommandOutput::error(format!("Failed to stop server '{id}': {e}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::console::render::Renderer;

    fn render(output: &CommandOutput) -> String {
        Renderer::new(false, None).render(output)
    }

    fn address(host: &str, port: u16) -> ServerAddress {
        ServerAddress {
            host: host.to_string(),
            port,
        }
    }

    fn view(
        id: &str,
        mode: ProxyMode,
        addresses: Vec<ServerAddress>,
        players: usize,
    ) -> ServerView {
        ServerView {
            config: ServerConfig::new(ServerId::new(id))
                .proxy_mode(mode)
                .addresses(addresses),
            players,
        }
    }

    fn fleet() -> Vec<ServerView> {
        let mut survival = view(
            "survival",
            ProxyMode::ClientOnly,
            vec![
                address("10.0.0.2", 25565),
                address("10.0.0.3", 25565),
                address("10.0.0.4", 25565),
            ],
            8,
        );
        survival.config.max_players = 100;
        vec![
            view(
                "lobby",
                ProxyMode::Passthrough,
                vec![address("10.0.0.1", 25565)],
                3,
            ),
            survival,
            view("creative", ProxyMode::Offline, Vec::new(), 0),
        ]
    }

    #[test]
    fn states_map_to_marked_spans() {
        assert_eq!(
            state_span(ServerState::Online),
            Span::marked(Mark::Up, "online")
        );
        assert_eq!(
            state_span(ServerState::Sleeping),
            Span::marked(Mark::Idle, "sleeping")
        );
        assert_eq!(
            state_span(ServerState::Starting),
            Span::marked(Mark::Busy, "starting")
        );
        assert_eq!(
            state_span(ServerState::Stopping),
            Span::marked(Mark::Busy, "stopping")
        );
        assert_eq!(
            state_span(ServerState::Crashed),
            Span::marked(Mark::Down, "crashed")
        );
        assert_eq!(state_span(ServerState::Unknown), Span::muted("unknown"));
    }

    #[test]
    fn modes_are_named_as_the_config_spells_them() {
        assert_eq!(mode_name(ProxyMode::ZeroCopy), "zero_copy");
        assert_eq!(mode_name(ProxyMode::ClientOnly), "client_only");
        assert_eq!(mode_name(ProxyMode::ServerOnly), "server_only");
    }

    #[test]
    fn servers_without_a_manager_skip_the_state_column() {
        assert_eq!(
            render(&servers_output(&fleet(), None)),
            "# Servers - 3 configured\n\
             | SERVER     PLAYERS   MODE          ADDRESS\n\
             | lobby      3         passthrough   10.0.0.1:25565\n\
             | survival   8/100     client_only   10.0.0.2:25565 +2\n\
             | creative   0         offline       -"
        );
    }

    #[test]
    fn managed_servers_show_their_state_and_the_counts() {
        let states = HashMap::from([
            ("survival".to_string(), ServerState::Online),
            ("creative".to_string(), ServerState::Crashed),
        ]);
        assert_eq!(
            render(&servers_output(&fleet(), Some(&states))),
            "# Servers - 3 configured, 1 online, 0 sleeping, 1 crashed\n\
             | SERVER     STATE     PLAYERS   MODE          ADDRESS\n\
             | lobby      -         3         passthrough   10.0.0.1:25565\n\
             | survival   online    8/100     client_only   10.0.0.2:25565 +2\n\
             | creative   crashed   0         offline       -"
        );
    }

    #[test]
    fn no_servers_is_a_note() {
        assert_eq!(
            render(&servers_output(&[], None)),
            "- No servers configured"
        );
    }

    #[test]
    fn a_server_lists_its_details() {
        let mut survival = fleet().remove(1);
        survival.config.domains = vec!["play.example.net".into(), "mc.example.net".into()];
        survival.config.network = Some("main".into());
        assert_eq!(
            render(&server_block(&survival, Some(ServerState::Sleeping)).into()),
            "# survival - sleeping\n\
             | addresses  10.0.0.2:25565, 10.0.0.3:25565, 10.0.0.4:25565\n\
             | domains    play.example.net, mc.example.net\n\
             | mode       client_only\n\
             | players    8 / 100\n\
             | network    main"
        );
    }

    #[test]
    fn an_unmanaged_server_has_no_meta() {
        let lobby = fleet().remove(0);
        assert_eq!(
            render(&server_block(&lobby, None).into()),
            "# lobby\n\
             | addresses  10.0.0.1:25565\n\
             | domains    -\n\
             | mode       passthrough\n\
             | players    3"
        );
    }
}
