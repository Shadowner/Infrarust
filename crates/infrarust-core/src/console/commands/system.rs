use std::io::IsTerminal;
use std::time::Duration;

use console::measure_text_width;
use infrarust_api::command::CommandInfo as PluginCommandInfo;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::proxy_control::{
    RestartCancelError, RestartOptions, RestartRequestError,
};
use infrarust_api::services::{
    AvailableUpdate, InstallRequest, InstallState, ProxyRestart, ProxyUpdates, RestartRequirement,
    UpdateRequestError, UpdateStatus,
};
use infrarust_protocol::version::ProtocolVersion;
use tokio_util::sync::CancellationToken;

use crate::console::ConsoleServices;
use crate::console::commands::servers::StateCounts;
use crate::console::commands::usage;
use crate::console::dispatcher::{ConsoleCommand, ConsoleCommandSpec};
use crate::console::output::{
    Block, CommandCategory, CommandOutput, Failure, Fields, Hint, Line, Node, Span, Table,
};
use crate::console::parser::format_duration_short;
use crate::console::render::usage_line;
use crate::terminal::Mark;
use crate::util::whole_seconds;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_POLL: Duration = Duration::from_millis(200);
const INSTALL_SETTLE: Duration = Duration::from_secs(2);

fn product_title() -> String {
    format!("Infrarust {VERSION}")
}

pub(crate) fn help_block(
    commands: &[&ConsoleCommandSpec],
    plugin_commands: &[PluginCommandInfo],
) -> Block {
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
            table.row([usage_line(info.usage), Span::muted(info.description).into()]);
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

    align_first_column(&mut block);
    block
}

fn align_first_column(block: &mut Block) {
    let text_width = |line: &Line| -> usize {
        line.0
            .iter()
            .map(|span| measure_text_width(&span.text))
            .sum()
    };
    let tables = |block: &Block| -> Vec<usize> {
        block
            .body
            .iter()
            .filter_map(|node| match node {
                Node::Table(table) => Some(
                    table
                        .rows
                        .iter()
                        .filter_map(|row| row.first())
                        .map(text_width)
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect()
    };
    let width = tables(block).into_iter().max().unwrap_or(0);
    for node in &mut block.body {
        if let Node::Table(table) = node {
            for cell in table.rows.iter_mut().filter_map(|row| row.first_mut()) {
                let missing = width - text_width(cell);
                if missing > 0 {
                    cell.0.push(Span::plain(" ".repeat(missing)));
                }
            }
        }
    }
}

pub(crate) fn command_block(info: &ConsoleCommandSpec) -> Block {
    let mut fields = Fields::new().field("usage", usage_line(info.usage));
    if !info.aliases.is_empty() {
        fields.push("aliases", info.aliases.join(", "));
    }
    fields.push("category", info.category.display_name());
    Block::new(info.name).meta(info.description).fields(fields)
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

pub(crate) fn version_block(update: Option<&AvailableUpdate>) -> Block {
    let mut fields = Fields::new();
    if let Some(range) = minecraft_range(ProtocolVersion::SUPPORTED) {
        fields.push("minecraft", range);
    }
    fields.push(
        "platform",
        format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
    );
    if let Some(update) = update {
        let text = format!("{} available", update.version);
        let line = if update.security {
            Line::from(Span::err(text)).push("  ").push(security_fix())
        } else {
            Span::warn(text).into()
        };
        fields.push("update", line);
    }
    Block::new(product_title()).fields(fields)
}

fn security_fix() -> Span {
    Span::err("security fix")
}

const fn restart_text(restart: RestartRequirement) -> &'static str {
    match restart {
        RestartRequirement::Hot => "hot restart",
        RestartRequirement::Full => "full restart",
        RestartRequirement::Manual => "stop and start the proxy by hand",
    }
}

fn update_meta(status: &UpdateStatus) -> Span {
    match (&status.available, &status.last_error) {
        (Some(update), _) if update.security => Span::err("security update available"),
        (Some(_), _) => Span::warn("update available"),
        (None, Some(_)) => Span::err("check failed"),
        (None, None) => Span::ok("up to date"),
    }
}

fn install_span(install: &InstallState) -> Option<Span> {
    match install {
        InstallState::Idle => None,
        InstallState::Running { version } => {
            Some(Span::marked(Mark::Busy, format!("installing {version}")))
        }
        InstallState::Installed { version } => Some(Span::ok(format!(
            "{version} installed, restart Infrarust to run it"
        ))),
        InstallState::Failed { reason } => Some(Span::err(format!("failed: {reason}"))),
    }
}

pub(crate) fn update_block(status: &UpdateStatus) -> Block {
    let mut fields = Fields::new();
    if let Some(update) = &status.available {
        let mut available = Line::from(Span::strong(update.version.as_str()));
        if update.security {
            available = available.push("  ").push(security_fix());
        }
        fields.push("available", available);
        if let Some(published) = &update.published_at {
            fields.push("published", published.as_str());
        }
        if let Some(notes) = &update.notes_url {
            fields.push("notes", notes.as_str());
        }
        fields.push("restart", restart_text(update.restart));
    }
    if let Some(install) = install_span(&status.install) {
        fields.push("install", install);
    }
    if status.binary_changed && !matches!(status.install, InstallState::Installed { .. }) {
        fields.push(
            "binary",
            Span::warn("changed on disk, restart Infrarust to run it"),
        );
    }
    if let Some(error) = &status.last_error {
        fields.push("error", Span::err(error.as_str()));
    }
    if let Some(refusal) = &status.install_refusal {
        fields.push("refused", Span::warn(refusal.as_str()));
    }

    let mut block =
        Block::new(format!("Infrarust {}", status.current_version)).meta(update_meta(status));
    if !fields.0.is_empty() {
        block = block.fields(fields);
    }
    let installable = matches!(
        status.install,
        InstallState::Idle | InstallState::Failed { .. }
    );
    if status.available.is_some() && status.install_refusal.is_none() && installable {
        block = block.gap().line(vec![
            Span::muted("type "),
            Span::plain("update install"),
            Span::muted(" to install it"),
        ]);
    }
    block
}

fn no_updates() -> Failure {
    Failure::new("Updates are unavailable").with_hint(Hint::Note(
        "Infrarust could not locate its own executable at startup".to_string(),
    ))
}

fn install_refused(error: &UpdateRequestError) -> Failure {
    let failure = Failure::new(format!("Cannot install: {error}"));
    match error {
        UpdateRequestError::NothingToInstall => {
            failure.with_hint(Hint::Note("type update to check for one".to_string()))
        }
        UpdateRequestError::Busy => {
            failure.with_hint(Hint::Note("type update to follow it".to_string()))
        }
        UpdateRequestError::Refused(_) => failure,
    }
}

fn install_output(install: &InstallState) -> CommandOutput {
    match install {
        InstallState::Installed { version } => {
            CommandOutput::Success(format!("Installed {version}, restart Infrarust to run it"))
        }
        InstallState::Failed { reason } => Failure::new(format!("Update failed: {reason}")).into(),
        InstallState::Idle | InstallState::Running { .. } => {
            CommandOutput::Note("The update is still installing, type update to follow it".into())
        }
    }
}

async fn advertised(updates: &dyn ProxyUpdates) -> Result<AvailableUpdate, CommandOutput> {
    if let Some(available) = updates.status().available {
        return Ok(available);
    }
    let status = updates.check_now().await;
    match (status.available, status.last_error) {
        (Some(available), _) => Ok(available),
        (None, Some(error)) => Err(Failure::new(format!("Update check failed: {error}")).into()),
        (None, None) => Err(CommandOutput::Note(format!(
            "Infrarust {} is up to date",
            status.current_version
        ))),
    }
}

async fn install(
    updates: &dyn ProxyUpdates,
    version: Option<&str>,
    shutdown: &CancellationToken,
) -> CommandOutput {
    if version.is_none()
        && let Err(output) = advertised(updates).await
    {
        return output;
    }

    let before = updates.status().install;
    let request = InstallRequest {
        version: version.map(str::to_string),
        then_restart: false,
    };
    if let Err(error) = updates.start_install(request) {
        return install_refused(&error).into();
    }
    tracing::info!(
        target: "console",
        version = version.unwrap_or("latest"),
        "Update install started from console"
    );

    match finished_install(updates, &before, shutdown).await {
        Some(install) => install_output(&install),
        None => CommandOutput::Note("The proxy is stopping before the update finished".into()),
    }
}

fn manual_restart_hint() -> Hint {
    Hint::Note("type update install, then stop the proxy and start it again".to_string())
}

async fn apply(
    updates: &dyn ProxyUpdates,
    restart: Option<&dyn ProxyRestart>,
    version: Option<&str>,
    shutdown: &CancellationToken,
) -> CommandOutput {
    if restart.is_none() {
        return Failure::new("Hot restart is unavailable here")
            .with_hint(manual_restart_hint())
            .into();
    }
    let available = match advertised(updates).await {
        Ok(available) => available,
        Err(output) => return output,
    };
    if let Some(version) = version
        && version.strip_prefix('v').unwrap_or(version) != available.version
    {
        return Failure::new(format!(
            "Only the advertised update {} can be applied in place",
            available.version
        ))
        .with_hint(Hint::Note(format!(
            "type update install {version}, then stop the proxy and start it again"
        )))
        .into();
    }
    if available.restart != RestartRequirement::Hot {
        return Failure::new(format!(
            "{} needs a {}, nothing was installed",
            available.version,
            restart_text(available.restart)
        ))
        .with_hint(manual_restart_hint())
        .into();
    }

    let before = updates.status().install;
    let request = InstallRequest {
        version: version.map(str::to_string),
        then_restart: true,
    };
    if let Err(error) = updates.start_install(request) {
        return install_refused(&error).into();
    }
    tracing::info!(
        target: "console",
        version = %available.version,
        "Update apply started from console"
    );

    match finished_install(updates, &before, shutdown).await {
        Some(InstallState::Installed { version }) => CommandOutput::Success(format!(
            "Installed {version}, restarting in place; follow the restart in the logs"
        )),
        Some(install) => install_output(&install),
        None => CommandOutput::Note("The proxy is stopping before the update finished".into()),
    }
}

fn restart_output(
    answer: Result<(), RestartRequestError>,
    delay: Option<Duration>,
) -> CommandOutput {
    match answer {
        Ok(()) => match delay.filter(|delay| !delay.is_zero()) {
            Some(delay) => CommandOutput::Success(format!(
                "Hot restart requested in {}, the players are told; 'restart cancel' calls it off",
                format_duration_short(whole_seconds(delay))
            )),
            None => {
                CommandOutput::Success("Hot restart requested, follow it in the logs".to_string())
            }
        },
        Err(RestartRequestError::Invalid(reason)) => {
            Failure::new(format!("The restart request is refused: {reason}")).into()
        }
        Err(RestartRequestError::InProgress) => {
            CommandOutput::Note("A restart is already in progress".to_string())
        }
        Err(RestartRequestError::ShuttingDown) => Failure::new("The proxy is shutting down").into(),
        Err(RestartRequestError::Unsupported(reason)) => {
            Failure::new(format!("Hot restart is unavailable: {reason}"))
                .with_hint(Hint::Note("stop the proxy and start it again".to_string()))
                .into()
        }
    }
}

async fn finished_install(
    updates: &dyn ProxyUpdates,
    before: &InstallState,
    shutdown: &CancellationToken,
) -> Option<InstallState> {
    let started = tokio::time::Instant::now();
    let mut seen_running = false;
    loop {
        match updates.status().install {
            InstallState::Running { .. } => seen_running = true,
            InstallState::Idle => {}
            install
                if seen_running || install != *before || started.elapsed() >= INSTALL_SETTLE =>
            {
                return Some(install);
            }
            _ => {}
        }
        tokio::select! {
            () = shutdown.cancelled() => return None,
            () = tokio::time::sleep(INSTALL_POLL) => {}
        }
    }
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

const HELP: ConsoleCommandSpec = ConsoleCommandSpec {
    name: "help",
    aliases: &["?"],
    description: "Show this help",
    usage: "help [command]",
    category: CommandCategory::System,
};

pub struct HelpCommand {
    commands: Vec<&'static ConsoleCommandSpec>,
}

impl HelpCommand {
    pub fn from_commands(mut commands: Vec<&'static ConsoleCommandSpec>) -> Self {
        commands.push(&HELP);
        Self { commands }
    }
}

impl ConsoleCommand for HelpCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &HELP
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let Some(name) = args.first() else {
                return help_block(&self.commands, &services.command_manager.list()).into();
            };
            let lower = name.to_lowercase();
            self.commands
                .iter()
                .find(|info| info.answers_to(&lower))
                .map_or_else(
                    || unknown_command(&lower).into(),
                    |info| command_block(info).into(),
                )
        })
    }
}

pub struct VersionCommand;

impl ConsoleCommand for VersionCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "version",
            aliases: &["ver"],
            description: "Show version info",
            usage: "version",
            category: CommandCategory::System,
        }
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let update = services
                .updates
                .as_ref()
                .and_then(|updates| updates.status().available);
            version_block(update.as_ref()).into()
        })
    }
}

pub struct UpdateCommand;

impl ConsoleCommand for UpdateCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "update",
            aliases: &[],
            description: "Check for, install or apply an update",
            usage: "update [install|apply [version]]",
            category: CommandCategory::System,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let Some(updates) = services.updates.as_deref() else {
                return no_updates().into();
            };
            match args {
                [] => update_block(&updates.check_now().await).into(),
                [action, rest @ ..]
                    if action.eq_ignore_ascii_case("install") && rest.len() <= 1 =>
                {
                    install(updates, rest.first().copied(), &services.shutdown).await
                }
                [action, rest @ ..] if action.eq_ignore_ascii_case("apply") && rest.len() <= 1 => {
                    apply(
                        updates,
                        services.restart.as_deref(),
                        rest.first().copied(),
                        &services.shutdown,
                    )
                    .await
                }
                _ => usage(self),
            }
        })
    }
}

pub struct RestartCommand;

impl ConsoleCommand for RestartCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "restart",
            aliases: &[],
            description: "Restart in place, keeping forwarded players connected",
            usage: "restart [delay] [message...] | restart cancel",
            category: CommandCategory::System,
        }
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let Some(restart) = services.restart.as_deref() else {
                return Failure::new("Hot restart is unavailable here")
                    .with_hint(Hint::Note("stop the proxy and start it again".to_string()))
                    .into();
            };
            if args == ["cancel"] {
                return match restart.cancel_restart() {
                    Ok(()) => {
                        tracing::info!(target: "console", "Hot restart cancelled from console");
                        CommandOutput::Success("The restart is cancelled".to_string())
                    }
                    Err(RestartCancelError::NothingScheduled) => {
                        CommandOutput::Note("No restart is scheduled".to_string())
                    }
                    Err(RestartCancelError::TooLate) => {
                        Failure::new("The restart is already under way").into()
                    }
                };
            }
            let (delay, message) = super::args::delay_and_message(args);
            tracing::info!(target: "console", "Hot restart requested from console");
            restart_output(
                restart.request_restart_with(RestartOptions { delay, message }),
                delay,
            )
        })
    }
}

pub struct StatusCommand;

impl ConsoleCommand for StatusCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "status",
            aliases: &["info"],
            description: "Proxy overview",
            usage: "status",
            category: CommandCategory::System,
        }
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            let report = StatusReport {
                uptime: format_duration_short(services.start_time.elapsed()),
                players: services.player_registry.online_count(),
                connections: services.connection_registry.count(),
                servers: services.config_service.get_all_server_configs().len(),
                managed: services.server_manager.as_ref().map(|manager| {
                    StateCounts::tally(manager.get_all_managed().iter().map(|(_, state)| state))
                }),
                plugins: services.plugins.plugin_count().await,
            };
            status_block(&report).into()
        })
    }
}

pub struct StopCommand;

impl ConsoleCommand for StopCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "stop",
            aliases: &["shutdown", "exit", "quit"],
            description: "Shutdown the proxy",
            usage: "stop",
            category: CommandCategory::System,
        }
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
        Box::pin(async move {
            tracing::info!(target: "console", "Proxy shutdown initiated from console");
            services.shutdown.cancel();
            CommandOutput::Success("Shutting down".to_string())
        })
    }
}

pub struct ClearCommand;

impl ConsoleCommand for ClearCommand {
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "clear",
            aliases: &["cls"],
            description: "Clear the screen",
            usage: "clear",
            category: CommandCategory::System,
        }
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        _services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
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
    fn spec(&self) -> &'static ConsoleCommandSpec {
        &ConsoleCommandSpec {
            name: "gc",
            aliases: &[],
            description: "Run garbage collection",
            usage: "gc",
            category: CommandCategory::System,
        }
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> BoxFuture<'a, CommandOutput> {
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

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    use infrarust_api::command::CommandSpec;
    use infrarust_api::services::ScheduledRestart;
    use infrarust_config::PermissionsConfig;

    use super::*;
    use crate::ban::manager::BanManager;
    use crate::console::render::Renderer;
    use crate::event_bus::EventBusImpl;
    use crate::permissions::PermissionService;
    use crate::player::registry::PlayerRegistryImpl;
    use crate::plugin::PluginManagerHandle;
    use crate::plugin::manager::PluginManager;
    use crate::registry::ConnectionRegistry;
    use crate::routing::DomainRouter;
    use crate::services::command_manager::CommandManagerImpl;
    use crate::services::config_service::ConfigServiceImpl;
    use crate::test_support::FakeUpdates;

    fn render(output: impl Into<CommandOutput>) -> String {
        Renderer::new(false, None).render(&output.into())
    }

    fn console(updates: Option<Arc<dyn ProxyUpdates>>) -> ConsoleServices {
        let connection_registry = Arc::new(ConnectionRegistry::new());
        ConsoleServices {
            player_registry: Arc::new(PlayerRegistryImpl::new(Arc::clone(&connection_registry))),
            ban_manager: Arc::new(BanManager::disabled(
                Arc::clone(&connection_registry),
                Arc::new(EventBusImpl::new()),
            )),
            connection_registry,
            server_manager: None,
            config_service: Arc::new(ConfigServiceImpl::new(
                Arc::new(DomainRouter::new()),
                PathBuf::from("infrarust.toml"),
                Arc::new(toml::from_str("").unwrap()),
            )),
            plugins: PluginManagerHandle::new(PluginManager::for_tests(Vec::new())),
            permission_service: Arc::new(
                PermissionService::new_sync(&PermissionsConfig::default()),
            ),
            command_manager: Arc::new(CommandManagerImpl::new()),
            shutdown: CancellationToken::new(),
            start_time: Instant::now(),
            updates,
            restart: None,
        }
    }

    async fn run(
        command: &dyn ConsoleCommand,
        args: &[&str],
        updates: &Arc<FakeUpdates>,
    ) -> String {
        let services = console(Some(Arc::clone(updates) as Arc<dyn ProxyUpdates>));
        render(command.execute(args, &services).await)
    }

    fn security_release() -> AvailableUpdate {
        AvailableUpdate {
            published_at: Some("2026-11-20T08:00:00Z".to_string()),
            notes_url: Some("https://example.invalid/v2.0.0-beta.6".to_string()),
            ..FakeUpdates::release("2.0.0-beta.6", true)
        }
    }

    fn offering(update: AvailableUpdate) -> UpdateStatus {
        UpdateStatus {
            available: Some(update),
            ..FakeUpdates::idle("2.0.0-beta.5")
        }
    }

    const DOCKER: &str = "this binary comes from a container image; update the image with `docker pull ghcr.io/shadowner/infrarust:2.0.0-beta.6` and recreate the container";

    #[tokio::test]
    async fn update_without_a_service_says_updates_are_unavailable() {
        let output = UpdateCommand.execute(&[], &console(None)).await;
        assert_eq!(
            render(output),
            "error: Updates are unavailable\n  Infrarust could not locate its own executable at startup"
        );
        let output = UpdateCommand.execute(&["install"], &console(None)).await;
        assert!(render(output).starts_with("error: Updates are unavailable"));
    }

    #[tokio::test]
    async fn update_checks_now_and_flags_a_security_release() {
        let updates = Arc::new(
            FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.5"))
                .on_check(offering(security_release())),
        );
        assert_eq!(
            run(&UpdateCommand, &[], &updates).await,
            "# Infrarust 2.0.0-beta.5 - security update available\n\
             | available  2.0.0-beta.6  security fix\n\
             | published  2026-11-20T08:00:00Z\n\
             | notes      https://example.invalid/v2.0.0-beta.6\n\
             | restart    full restart\n\
             |\n\
             | type update install to install it"
        );
        assert_eq!(updates.checks(), 1);
    }

    #[tokio::test]
    async fn update_says_when_the_proxy_is_up_to_date() {
        let updates = Arc::new(FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.6")));
        assert_eq!(
            run(&UpdateCommand, &[], &updates).await,
            "# Infrarust 2.0.0-beta.6 - up to date"
        );
    }

    #[tokio::test]
    async fn update_shows_why_the_check_failed() {
        let updates = Arc::new(FakeUpdates::new(UpdateStatus {
            last_error: Some("the update server rate limit is reached".to_string()),
            ..FakeUpdates::idle("2.0.0-beta.5")
        }));
        assert_eq!(
            run(&UpdateCommand, &[], &updates).await,
            "# Infrarust 2.0.0-beta.5 - check failed\n\
             | error  the update server rate limit is reached"
        );
    }

    #[tokio::test]
    async fn update_shows_the_install_refusal_instead_of_the_install_hint() {
        let updates = Arc::new(FakeUpdates::new(UpdateStatus {
            install_refusal: Some(DOCKER.to_string()),
            ..offering(FakeUpdates::release("2.0.0-beta.6", false))
        }));
        assert_eq!(
            run(&UpdateCommand, &[], &updates).await,
            format!(
                "# Infrarust 2.0.0-beta.5 - update available\n\
                 | available  2.0.0-beta.6\n\
                 | restart    full restart\n\
                 | refused    {DOCKER}"
            )
        );
    }

    #[tokio::test]
    async fn update_shows_an_installed_release_waiting_for_a_restart() {
        let updates = Arc::new(FakeUpdates::new(UpdateStatus {
            install: InstallState::Installed {
                version: "2.0.0-beta.6".to_string(),
            },
            binary_changed: true,
            ..offering(FakeUpdates::release("2.0.0-beta.6", false))
        }));
        assert_eq!(
            run(&UpdateCommand, &[], &updates).await,
            "# Infrarust 2.0.0-beta.5 - update available\n\
             | available  2.0.0-beta.6\n\
             | restart    full restart\n\
             | install    2.0.0-beta.6 installed, restart Infrarust to run it"
        );
    }

    #[tokio::test]
    async fn update_install_waits_for_the_installed_version() {
        let updates = Arc::new(
            FakeUpdates::new(offering(FakeUpdates::release("2.0.0-beta.6", false))).installs(
                InstallState::Installed {
                    version: "2.0.0-beta.6".to_string(),
                },
            ),
        );
        assert_eq!(
            run(&UpdateCommand, &["install"], &updates).await,
            "ok: Installed 2.0.0-beta.6, restart Infrarust to run it"
        );
        assert_eq!(updates.requests(), [InstallRequest::default()]);
        assert_eq!(updates.checks(), 0);
    }

    #[tokio::test]
    async fn update_install_of_a_version_reports_the_failure_and_its_instructions() {
        let reason = "the binary rejects the configuration infrarust.toml: unknown field `update`";
        let updates = Arc::new(
            FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.5")).installs(InstallState::Failed {
                reason: reason.to_string(),
            }),
        );
        assert_eq!(
            run(&UpdateCommand, &["INSTALL", "2.0.0-beta.4"], &updates).await,
            format!("error: Update failed: {reason}")
        );
        assert_eq!(
            updates.requests(),
            [InstallRequest {
                version: Some("2.0.0-beta.4".to_string()),
                then_restart: false,
            }]
        );
        assert_eq!(updates.checks(), 0);
    }

    #[tokio::test]
    async fn update_install_shows_a_refusal_with_its_instructions() {
        let updates = Arc::new(
            FakeUpdates::new(offering(FakeUpdates::release("2.0.0-beta.6", false)))
                .refuses(UpdateRequestError::Refused(DOCKER.to_string())),
        );
        assert_eq!(
            run(&UpdateCommand, &["install"], &updates).await,
            format!("error: Cannot install: {DOCKER}")
        );
    }

    #[tokio::test]
    async fn update_install_checks_first_when_no_update_is_known() {
        let updates = Arc::new(FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.6")));
        assert_eq!(
            run(&UpdateCommand, &["install"], &updates).await,
            "- Infrarust 2.0.0-beta.6 is up to date"
        );
        assert_eq!(updates.checks(), 1);
        assert!(updates.requests().is_empty());

        let found = Arc::new(
            FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.5"))
                .on_check(offering(security_release()))
                .installs(InstallState::Installed {
                    version: "2.0.0-beta.6".to_string(),
                }),
        );
        assert_eq!(
            run(&UpdateCommand, &["install"], &found).await,
            "ok: Installed 2.0.0-beta.6, restart Infrarust to run it"
        );
        assert_eq!(found.checks(), 1);
    }

    #[tokio::test]
    async fn update_rejects_unknown_arguments_with_its_usage() {
        let updates = Arc::new(FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.6")));
        for args in [&["rollback"][..], &["install", "1.0.0", "now"]] {
            assert_eq!(
                run(&UpdateCommand, args, &updates).await,
                "error: Missing arguments\n  usage: update [install|apply [version]]"
            );
        }
        assert!(updates.requests().is_empty());
    }

    struct ScriptedRestart {
        answer: Result<(), RestartRequestError>,
        cancel: Result<(), RestartCancelError>,
        asked: std::sync::Mutex<Vec<RestartOptions>>,
    }

    impl infrarust_api::__sealed::Sealed for ScriptedRestart {}

    impl ProxyRestart for ScriptedRestart {
        fn request_restart_with(&self, options: RestartOptions) -> Result<(), RestartRequestError> {
            self.asked.lock().unwrap().push(options);
            self.answer.clone()
        }

        fn cancel_restart(&self) -> Result<(), RestartCancelError> {
            self.cancel
        }

        fn scheduled_restart(&self) -> Option<ScheduledRestart> {
            None
        }
    }

    impl ScriptedRestart {
        fn asked(&self) -> Vec<RestartOptions> {
            self.asked.lock().unwrap().clone()
        }

        fn handle(self: &Arc<Self>) -> Arc<dyn ProxyRestart> {
            Arc::clone(self) as Arc<dyn ProxyRestart>
        }
    }

    fn console_restarting(restart: &Arc<ScriptedRestart>) -> ConsoleServices {
        console_with(None, Some(restart.handle()))
    }

    fn answering(answer: Result<(), RestartRequestError>) -> Arc<ScriptedRestart> {
        cancelling(answer, Err(RestartCancelError::NothingScheduled))
    }

    fn cancelling(
        answer: Result<(), RestartRequestError>,
        cancel: Result<(), RestartCancelError>,
    ) -> Arc<ScriptedRestart> {
        Arc::new(ScriptedRestart {
            answer,
            cancel,
            asked: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn console_with(
        updates: Option<Arc<dyn ProxyUpdates>>,
        restart: Option<Arc<dyn ProxyRestart>>,
    ) -> ConsoleServices {
        ConsoleServices {
            restart,
            ..console(updates)
        }
    }

    fn hot_release() -> AvailableUpdate {
        AvailableUpdate {
            restart: RestartRequirement::Hot,
            ..FakeUpdates::release("2.0.0-beta.6", false)
        }
    }

    async fn apply_with(
        args: &[&str],
        updates: &Arc<FakeUpdates>,
        restart: Option<Arc<dyn ProxyRestart>>,
    ) -> String {
        let services = console_with(Some(Arc::clone(updates) as Arc<dyn ProxyUpdates>), restart);
        render(UpdateCommand.execute(args, &services).await)
    }

    #[tokio::test]
    async fn restart_without_a_handle_says_hot_restart_is_unavailable() {
        assert_eq!(
            render(RestartCommand.execute(&[], &console(None)).await),
            "error: Hot restart is unavailable here\n  stop the proxy and start it again"
        );
    }

    #[tokio::test]
    async fn restart_reports_how_the_request_was_answered() {
        for (answer, expected) in [
            (Ok(()), "ok: Hot restart requested, follow it in the logs"),
            (
                Err(RestartRequestError::InProgress),
                "- A restart is already in progress",
            ),
            (
                Err(RestartRequestError::ShuttingDown),
                "error: The proxy is shutting down",
            ),
            (
                Err(RestartRequestError::Unsupported("no restart loop".into())),
                "error: Hot restart is unavailable: no restart loop\n  stop the proxy and start it again",
            ),
            (
                Err(RestartRequestError::Invalid("the delay is 2h".into())),
                "error: The restart request is refused: the delay is 2h",
            ),
        ] {
            let restart = answering(answer);
            let services = console_restarting(&restart);
            assert_eq!(
                render(RestartCommand.execute(&[], &services).await),
                expected
            );
            assert_eq!(restart.asked(), [RestartOptions::default()]);
        }
    }

    #[tokio::test]
    async fn restart_takes_a_delay_and_a_message() {
        for (args, delay, message, expected) in [
            (
                &["30s", "&eBack", "in", "{delay}"][..],
                Some(Duration::from_secs(30)),
                Some("&eBack in {delay}"),
                "ok: Hot restart requested in 30s, the players are told; 'restart cancel' calls it off",
            ),
            (
                &["2m"][..],
                Some(Duration::from_secs(120)),
                None,
                "ok: Hot restart requested in 2m, the players are told; 'restart cancel' calls it off",
            ),
            (
                &["500ms"][..],
                Some(Duration::from_millis(500)),
                None,
                "ok: Hot restart requested in 1s, the players are told; 'restart cancel' calls it off",
            ),
            (
                &["0s"][..],
                Some(Duration::ZERO),
                None,
                "ok: Hot restart requested, follow it in the logs",
            ),
            (
                &["Maintenance", "now"][..],
                None,
                Some("Maintenance now"),
                "ok: Hot restart requested, follow it in the logs",
            ),
            (
                &[][..],
                None,
                None,
                "ok: Hot restart requested, follow it in the logs",
            ),
        ] {
            let restart = answering(Ok(()));
            let services = console_restarting(&restart);
            assert_eq!(
                render(RestartCommand.execute(args, &services).await),
                expected
            );
            assert_eq!(
                restart.asked(),
                [RestartOptions {
                    delay,
                    message: message.map(str::to_string)
                }]
            );
        }
    }

    #[tokio::test]
    async fn restart_cancel_reports_what_was_called_off() {
        for (cancel, expected) in [
            (Ok(()), "ok: The restart is cancelled"),
            (
                Err(RestartCancelError::NothingScheduled),
                "- No restart is scheduled",
            ),
            (
                Err(RestartCancelError::TooLate),
                "error: The restart is already under way",
            ),
        ] {
            let restart = cancelling(Ok(()), cancel);
            let services = console_restarting(&restart);
            assert_eq!(
                render(RestartCommand.execute(&["cancel"], &services).await),
                expected
            );
            assert!(restart.asked().is_empty());
        }
    }

    #[tokio::test]
    async fn update_apply_without_hot_restart_installs_nothing() {
        let updates = Arc::new(FakeUpdates::new(offering(hot_release())));
        assert_eq!(
            apply_with(&["apply"], &updates, None).await,
            "error: Hot restart is unavailable here\n  type update install, then stop the proxy and start it again"
        );
        assert!(updates.requests().is_empty());
    }

    #[tokio::test]
    async fn update_apply_of_a_release_needing_a_full_restart_installs_nothing() {
        let updates = Arc::new(FakeUpdates::new(offering(FakeUpdates::release(
            "2.0.0-beta.6",
            false,
        ))));
        let restart = answering(Ok(()));
        assert_eq!(
            apply_with(&["apply"], &updates, Some(restart.handle())).await,
            "error: 2.0.0-beta.6 needs a full restart, nothing was installed\n  type update install, then stop the proxy and start it again"
        );
        assert!(updates.requests().is_empty());
        assert!(restart.asked().is_empty());
    }

    #[tokio::test]
    async fn update_apply_installs_a_hot_release_and_asks_for_the_restart() {
        let updates = Arc::new(FakeUpdates::new(offering(hot_release())).installs(
            InstallState::Installed {
                version: "2.0.0-beta.6".to_string(),
            },
        ));
        let restart: Arc<dyn ProxyRestart> = answering(Ok(()));
        assert_eq!(
            apply_with(&["APPLY"], &updates, Some(Arc::clone(&restart))).await,
            "ok: Installed 2.0.0-beta.6, restarting in place; follow the restart in the logs"
        );
        assert_eq!(
            updates.requests(),
            [InstallRequest {
                version: None,
                then_restart: true,
            }]
        );
        assert_eq!(
            apply_with(&["apply", "v2.0.0-beta.6"], &updates, Some(restart)).await,
            "ok: Installed 2.0.0-beta.6, restarting in place; follow the restart in the logs"
        );
        assert_eq!(
            updates.requests()[1].version.as_deref(),
            Some("v2.0.0-beta.6")
        );
    }

    #[tokio::test]
    async fn update_apply_only_takes_the_advertised_release() {
        let updates = Arc::new(FakeUpdates::new(offering(hot_release())));
        let restart: Arc<dyn ProxyRestart> = answering(Ok(()));
        assert_eq!(
            apply_with(
                &["apply", "2.0.0-beta.4"],
                &updates,
                Some(Arc::clone(&restart))
            )
            .await,
            "error: Only the advertised update 2.0.0-beta.6 can be applied in place\n  type update install 2.0.0-beta.4, then stop the proxy and start it again"
        );
        assert!(updates.requests().is_empty());

        let current = Arc::new(FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.6")));
        assert_eq!(
            apply_with(&["apply"], &current, Some(restart)).await,
            "- Infrarust 2.0.0-beta.6 is up to date"
        );
        assert_eq!(current.checks(), 1);
    }

    #[tokio::test]
    async fn waiting_for_an_install_stops_with_the_proxy() {
        let updates = FakeUpdates::new(offering(FakeUpdates::release("2.0.0-beta.6", false)))
            .installs(InstallState::Installed {
                version: "2.0.0-beta.6".to_string(),
            });
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        assert_eq!(
            render(install(&updates, None, &shutdown).await),
            "- The proxy is stopping before the update finished"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_install_ending_like_the_previous_one_is_reported_after_a_pause() {
        let failed = InstallState::Failed {
            reason: "request to https://example.invalid failed".to_string(),
        };
        let updates = FakeUpdates::new(UpdateStatus {
            install: failed.clone(),
            ..FakeUpdates::idle("2.0.0-beta.5")
        });
        let started = tokio::time::Instant::now();
        let finished = finished_install(&updates, &failed, &CancellationToken::new()).await;
        assert_eq!(finished, Some(failed));
        assert!(started.elapsed() >= INSTALL_SETTLE);
    }

    #[test]
    fn version_flags_a_pending_security_update() {
        let rendered = render(version_block(Some(&security_release())));
        assert!(
            rendered.ends_with("| update     2.0.0-beta.6 available  security fix"),
            "{rendered}"
        );
        let rendered = render(version_block(Some(&FakeUpdates::release(
            "2.0.0-beta.6",
            false,
        ))));
        assert!(
            rendered.ends_with("| update     2.0.0-beta.6 available"),
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn version_reads_the_cached_status_without_checking() {
        let updates = Arc::new(FakeUpdates::new(offering(FakeUpdates::release(
            "2.0.0-beta.6",
            false,
        ))));
        let rendered = run(&VersionCommand, &[], &updates).await;
        assert!(rendered.ends_with("2.0.0-beta.6 available"), "{rendered}");
        assert_eq!(updates.checks(), 0);

        let rendered = render(VersionCommand.execute(&[], &console(None)).await);
        assert_eq!(rendered, render(version_block(None)));
    }

    fn info(
        name: &'static str,
        aliases: &'static [&'static str],
        description: &'static str,
        usage: &'static str,
        category: CommandCategory,
    ) -> &'static ConsoleCommandSpec {
        Box::leak(Box::new(ConsoleCommandSpec {
            name,
            aliases,
            description,
            usage,
            category,
        }))
    }

    fn builtins() -> Vec<&'static ConsoleCommandSpec> {
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
    fn help_aligns_descriptions_across_categories() {
        assert_eq!(
            render(help_block(&builtins(), &[])),
            "# Commands - 4 built-in, help <command> for details\n\
             | PLAYERS\n\
             | list [server]                         List online players\n\
             | kick <player> [reason...]             Kick a player\n\
             |\n\
             | BANS\n\
             | ban <player> [duration] [reason...]   Ban a player\n\
             |\n\
             | SYSTEM\n\
             | status                                Proxy overview"
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
             | status              Proxy overview\n\
             |\n\
             | PLUGIN COMMANDS\n\
             | lobby (hub:lobby)   Go to the lobby"
        );
    }

    #[test]
    fn help_for_one_command_shows_its_usage_and_aliases() {
        assert_eq!(
            render(command_block(builtins()[2])),
            "# ban - Ban a player\n\
             | usage     ban <player> [duration] [reason...]\n\
             | category  Bans"
        );
        assert_eq!(
            render(command_block(builtins()[0])),
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
            render(version_block(None)),
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
        let Node::Fields(fields) = &block.body[0] else {
            panic!("status is a list of fields");
        };
        let (_, servers) = &fields.0[2];
        assert_eq!(servers.0[2], Span::marked(Mark::Up, "1 online"));
        assert_eq!(servers.0[6], Span::marked(Mark::Down, "2 crashed"));
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
