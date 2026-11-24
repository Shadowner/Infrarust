//! Built-in proxy commands (`/infrarust`, `/ir`).

pub mod actions;
pub mod brigadier;
pub(crate) mod subcommands;

use std::sync::{Arc, LazyLock};
use std::time::Instant;

use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::{
    CommandContext, CommandHandler, CommandSource, CommandSpec, SuggestContext, Suggestion,
};
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ProxyUpdates;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::plugin_registry::PluginRegistry;

use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::server::ProxyServices;
use crate::services::command_manager::CommandManagerImpl;
use crate::services::config_service::ConfigServiceImpl;
use infrarust_server_manager::ServerManagerService;

const NO_PERMISSION: &str = "You don't have permission.";

pub(crate) trait SubcommandHandler: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn usage(&self) -> &str;

    fn aliases(&self) -> &'static [SubcommandAlias] {
        &[]
    }

    fn admin_only(&self) -> bool {
        false
    }

    fn runs_bare(&self) -> bool {
        true
    }

    fn arguments(&self) -> &'static [ArgSpec] {
        &[]
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()>;

    fn tab_complete<'a>(
        &'a self,
        _args: &'a [String],
        _source: &'a CommandSource,
        _services: &'a CommandServices,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async { Vec::new() })
    }
}

pub(crate) struct CommandServices {
    pub player_registry: Arc<PlayerRegistryImpl>,
    pub config_service: Arc<ConfigServiceImpl>,
    pub server_manager: Option<Arc<ServerManagerService>>,
    pub plugin_registry: Arc<dyn PluginRegistry>,
    pub command_manager: Arc<CommandManagerImpl>,
    pub permission_service: Arc<PermissionService>,
    pub start_time: Instant,
    pub updates: Option<Arc<dyn ProxyUpdates>>,
}

impl CommandServices {
    pub(crate) fn complete_player_names(&self, prefix: &str) -> Vec<String> {
        let prefix = prefix.to_lowercase();
        self.player_registry
            .get_all_players()
            .into_iter()
            .map(|player| player.profile().username.clone())
            .filter(|name| name.to_lowercase().starts_with(&prefix))
            .collect()
    }

    pub(crate) fn complete_server_names(&self, prefix: &str) -> Vec<String> {
        self.config_service
            .get_all_server_configs()
            .into_iter()
            .map(|config| config.id.as_str().to_string())
            .filter(|name| name.starts_with(prefix))
            .collect()
    }
}

pub(crate) struct SubcommandAlias {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) usage: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) enum ArgParser {
    Word,
    Phrase,
}

pub(crate) struct ArgSpec {
    pub(crate) name: &'static str,
    pub(crate) parser: ArgParser,
    pub(crate) executable: bool,
    pub(crate) ask_server: bool,
}

impl ArgSpec {
    pub(crate) const fn word(name: &'static str) -> Self {
        Self {
            name,
            parser: ArgParser::Word,
            executable: true,
            ask_server: true,
        }
    }

    pub(crate) const fn phrase(name: &'static str) -> Self {
        Self {
            name,
            parser: ArgParser::Phrase,
            executable: true,
            ask_server: false,
        }
    }

    pub(crate) const fn needs_more(mut self) -> Self {
        self.executable = false;
        self
    }

    pub(crate) const fn ask_server(mut self) -> Self {
        self.ask_server = true;
        self
    }
}

pub(crate) struct Entry<'a> {
    pub(crate) name: &'a str,
    pub(crate) description: &'a str,
    pub(crate) usage: &'a str,
    pub(crate) is_alias: bool,
    pub(crate) handler: &'a dyn SubcommandHandler,
}

pub(crate) struct Subcommands(Vec<Box<dyn SubcommandHandler>>);

impl Subcommands {
    pub(crate) fn entries(&self) -> impl Iterator<Item = Entry<'_>> {
        self.0.iter().flat_map(|handler| {
            let handler = handler.as_ref();
            let own = Entry {
                name: handler.name(),
                description: handler.description(),
                usage: handler.usage(),
                is_alias: false,
                handler,
            };
            let aliases = handler.aliases().iter().map(move |alias| Entry {
                name: alias.name,
                description: alias.description,
                usage: alias.usage,
                is_alias: true,
                handler,
            });
            std::iter::once(own).chain(aliases)
        })
    }

    pub(crate) fn entry(&self, name: &str) -> Option<Entry<'_>> {
        self.entries()
            .filter(|entry| entry.name == name)
            .min_by_key(|entry| entry.is_alias)
    }
}

pub(crate) static SUBCOMMANDS: LazyLock<Subcommands> = LazyLock::new(|| {
    Subcommands(vec![
        Box::new(subcommands::help::HelpSubcommand),
        Box::new(subcommands::version::VersionSubcommand),
        Box::new(subcommands::list::ListSubcommand),
        Box::new(subcommands::reload::ReloadSubcommand),
        Box::new(subcommands::server::ServerSubcommand),
        Box::new(subcommands::find::FindSubcommand),
        Box::new(subcommands::send::SendSubcommand),
        Box::new(subcommands::kick::KickSubcommand),
        Box::new(subcommands::broadcast::BroadcastSubcommand),
        Box::new(subcommands::plugin::PluginSubcommand),
    ])
});

struct InfrarustRootCommand {
    services: Arc<CommandServices>,
}

impl InfrarustRootCommand {
    const fn new(services: Arc<CommandServices>) -> Self {
        Self { services }
    }

    fn permission_entries() -> impl Iterator<Item = (&'static str, &'static str, bool)> {
        SUBCOMMANDS
            .entries()
            .map(|entry| (entry.name, entry.description, entry.handler.admin_only()))
    }

    fn allowed(&self, name: &str, source: &CommandSource) -> bool {
        self.services
            .permission_service
            .is_command_allowed(name, source)
    }

    async fn complete(&self, partial_args: &[String], source: &CommandSource) -> Vec<String> {
        match partial_args.len() {
            0 | 1 => {
                let prefix = partial_args.first().map(String::as_str).unwrap_or("");
                SUBCOMMANDS
                    .entries()
                    .map(|entry| entry.name)
                    .filter(|name| name.starts_with(prefix))
                    .filter(|name| self.allowed(name, source))
                    .map(str::to_string)
                    .collect()
            }
            _ => {
                let sub_name = partial_args[0].to_lowercase();
                if !self.allowed(&sub_name, source) {
                    return vec![];
                }
                match SUBCOMMANDS.entry(&sub_name) {
                    Some(entry) if !entry.is_alias => {
                        entry
                            .handler
                            .tab_complete(&partial_args[1..], source, &self.services)
                            .await
                    }
                    _ => vec![],
                }
            }
        }
    }
}

impl CommandHandler for InfrarustRootCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let sub_name = ctx.args.first().map(|s| s.to_lowercase());
            let allowed = match sub_name.as_deref() {
                Some(name) => self.allowed(name, &ctx.source),
                None => !self
                    .services
                    .permission_service
                    .visible_subcommands(&ctx.source)
                    .is_empty(),
            };
            if !allowed {
                ctx.source.send_message(ProxyMessage::error(NO_PERMISSION));
                return;
            }

            match sub_name.and_then(|name| SUBCOMMANDS.entry(&name)) {
                Some(entry) if entry.is_alias => {
                    entry.handler.execute(&ctx, &[], &self.services).await;
                }
                Some(entry) => {
                    let remaining_args: Vec<String> = ctx.args.iter().skip(1).cloned().collect();
                    entry
                        .handler
                        .execute(&ctx, &remaining_args, &self.services)
                        .await;
                }
                None => subcommands::help::handle_help(&ctx, &[], &self.services),
            }
        })
    }

    fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        Box::pin(async move {
            self.complete(&ctx.args, &ctx.source)
                .await
                .into_iter()
                .map(Suggestion::new)
                .collect()
        })
    }
}

/// Registers the built-in `/infrarust` (alias `/ir`) command.
pub fn register_builtin_commands(
    command_manager: &CommandManagerImpl,
    proxy_services: &ProxyServices,
    plugin_registry: Arc<dyn PluginRegistry>,
    start_time: Instant,
    updates: Option<Arc<dyn ProxyUpdates>>,
) {
    let services = Arc::new(CommandServices {
        player_registry: Arc::clone(&proxy_services.player_registry),
        config_service: Arc::new(ConfigServiceImpl::new(
            Arc::clone(&proxy_services.domain_router),
            proxy_services.config_path.clone(),
            Arc::clone(&proxy_services.config),
        )),
        server_manager: proxy_services.server_manager.clone(),
        plugin_registry,
        command_manager: Arc::clone(&proxy_services.command_manager),
        permission_service: Arc::clone(&proxy_services.permission_service),
        start_time,
        updates,
    });

    let root_cmd = InfrarustRootCommand::new(services);

    proxy_services
        .permission_service
        .register_subcommands(InfrarustRootCommand::permission_entries());

    command_manager.register_builtin(
        CommandSpec::new("infrarust")
            .alias("ir")
            .description("Infrarust proxy commands")
            .usage("/ir <subcommand>"),
        Box::new(root_cmd),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::time::Instant;

    use infrarust_api::permissions::{DefaultPermissionChecker, PermissionMap};
    use infrarust_config::PermissionsConfig;

    use crate::permissions::PermissionService;
    use crate::player::registry::PlayerRegistryImpl;
    use crate::plugin::PluginRegistryImpl;
    use crate::registry::ConnectionRegistry;
    use crate::routing::DomainRouter;
    use crate::services::command_manager::CommandManagerImpl;
    use crate::services::config_service::ConfigServiceImpl;

    fn root_command(player_commands: &[&str]) -> InfrarustRootCommand {
        let permission_service = Arc::new(PermissionService::new_sync(&PermissionsConfig {
            player_commands: player_commands.iter().map(|s| (*s).to_string()).collect(),
            ..PermissionsConfig::default()
        }));

        let services = Arc::new(CommandServices {
            player_registry: Arc::new(PlayerRegistryImpl::new(Arc::new(ConnectionRegistry::new()))),
            config_service: Arc::new(ConfigServiceImpl::new(
                Arc::new(DomainRouter::new()),
                std::path::PathBuf::from("infrarust.toml"),
                Arc::new(toml::from_str("").expect("empty proxy config is fully defaulted")),
            )),
            server_manager: None,
            plugin_registry: Arc::new(PluginRegistryImpl::new()),
            command_manager: Arc::new(CommandManagerImpl::new()),
            permission_service: Arc::clone(&permission_service),
            start_time: Instant::now(),
            updates: None,
        });

        let root = InfrarustRootCommand::new(services);
        permission_service.register_subcommands(InfrarustRootCommand::permission_entries());
        root
    }

    fn source(root: &InfrarustRootCommand, checker: PermissionMap) -> CommandSource {
        CommandSource::console(root.services.permission_service.resolved(Arc::new(checker)))
    }

    fn player(root: &InfrarustRootCommand) -> CommandSource {
        CommandSource::console(
            root.services
                .permission_service
                .resolved(Arc::new(DefaultPermissionChecker)),
        )
    }

    #[tokio::test]
    async fn tab_complete_hides_admin_subcommands_from_players() {
        let root = root_command(&["list", "help", "kick"]);
        let names = root.complete(&[String::new()], &player(&root)).await;

        assert!(names.contains(&"list".to_string()));
        assert!(names.contains(&"help".to_string()));
        assert!(
            !names.contains(&"kick".to_string()),
            "non-admin must not see the admin-only 'kick' subcommand"
        );
    }

    #[tokio::test]
    async fn tab_complete_shows_admin_subcommands_to_admins() {
        let root = root_command(&["list", "help"]);
        let admin = source(
            &root,
            PermissionMap::new().with(infrarust_api::permissions::ADMIN_PERMISSION, true),
        );
        let names = root.complete(&[String::new()], &admin).await;

        assert!(names.contains(&"kick".to_string()));
        assert!(names.contains(&"list".to_string()));
    }

    #[tokio::test]
    async fn tab_complete_does_not_descend_into_forbidden_subcommand() {
        let root = root_command(&["list", "help"]);
        let suggestions = root
            .complete(&["kick".to_string(), String::new()], &player(&root))
            .await;

        assert!(
            suggestions.is_empty(),
            "non-admin must not receive argument completions for an admin command"
        );
    }

    #[tokio::test]
    async fn a_granted_node_opens_one_admin_subcommand() {
        let root = root_command(&[]);
        let moderator = source(
            &root,
            PermissionMap::new().with("infrarust.command.kick", true),
        );
        let names = root.complete(&[String::new()], &moderator).await;

        assert_eq!(names, ["kick"]);
    }
}
