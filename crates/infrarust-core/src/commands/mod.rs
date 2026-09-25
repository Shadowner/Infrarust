//! Built-in proxy commands (`/infrarust`, `/ir`).

pub mod brigadier;
mod subcommands;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use infrarust_api::command::{
    CommandContext, CommandHandler, CommandSource, CommandSpec, SuggestContext, Suggestion,
};
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::services::plugin_registry::PluginRegistry;

use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::services::ProxyServices;
use crate::services::command_manager::CommandManagerImpl;
use crate::services::config_service::ConfigServiceImpl;
use infrarust_server_manager::ServerManagerService;

const NO_PERMISSION: &str = "You don't have permission.";

pub(crate) trait SubcommandHandler: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn usage(&self) -> &str;

    fn admin_only(&self) -> bool {
        false
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
}

struct InfrarustRootCommand {
    subcommands: HashMap<String, Box<dyn SubcommandHandler>>,
    services: Arc<CommandServices>,
}

impl InfrarustRootCommand {
    fn new(services: Arc<CommandServices>) -> Self {
        let mut subcommands: HashMap<String, Box<dyn SubcommandHandler>> = HashMap::new();

        let sub_list: Vec<Box<dyn SubcommandHandler>> = vec![
            Box::new(subcommands::help::HelpSubcommand),
            Box::new(subcommands::version::VersionSubcommand),
            Box::new(subcommands::list::ListSubcommand),
            Box::new(subcommands::server::ServerSubcommand),
            Box::new(subcommands::find::FindSubcommand),
            Box::new(subcommands::send::SendSubcommand),
            Box::new(subcommands::broadcast::BroadcastSubcommand),
            Box::new(subcommands::kick::KickSubcommand),
            Box::new(subcommands::plugins::PluginsSubcommand),
            Box::new(subcommands::plugin::PluginSubcommand),
            Box::new(subcommands::reload::ReloadSubcommand),
        ];

        for sub in sub_list {
            subcommands.insert(sub.name().to_string(), sub);
        }

        Self {
            subcommands,
            services,
        }
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
                self.subcommands
                    .keys()
                    .filter(|name| name.starts_with(prefix))
                    .filter(|name| self.allowed(name, source))
                    .cloned()
                    .collect()
            }
            _ => {
                let sub_name = partial_args[0].to_lowercase();
                if !self.allowed(&sub_name, source) {
                    return vec![];
                }
                if let Some(sub) = self.subcommands.get(&sub_name) {
                    sub.tab_complete(&partial_args[1..], source, &self.services)
                        .await
                } else {
                    vec![]
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

            match sub_name.as_deref() {
                Some("help") => {
                    let remaining_args: Vec<String> = ctx.args.iter().skip(1).cloned().collect();
                    subcommands::help::handle_help(
                        &ctx,
                        &remaining_args,
                        &self.subcommands,
                        &self.services,
                    );
                }
                Some(name) if self.subcommands.contains_key(name) => {
                    let remaining_args: Vec<String> = ctx.args.iter().skip(1).cloned().collect();
                    self.subcommands[name]
                        .execute(&ctx, &remaining_args, &self.services)
                        .await;
                }
                _ => {
                    subcommands::help::handle_help(&ctx, &[], &self.subcommands, &self.services);
                }
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
    });

    let root_cmd = InfrarustRootCommand::new(services);

    proxy_services.permission_service.register_subcommands(
        root_cmd
            .subcommands
            .values()
            .map(|sub| (sub.name(), sub.description(), sub.admin_only())),
    );

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
        });

        let root = InfrarustRootCommand::new(services);
        permission_service.register_subcommands(
            root.subcommands
                .values()
                .map(|sub| (sub.name(), sub.description(), sub.admin_only())),
        );
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
