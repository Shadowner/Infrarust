//! Built-in proxy commands (`/infrarust`, `/ir`).

pub mod brigadier;
mod subcommands;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use infrarust_api::command::{CommandContext, CommandHandler};
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::services::player_registry::PlayerRegistry;
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

    fn required_level(&self) -> PermissionLevel {
        PermissionLevel::Player
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()>;

    fn tab_complete(&self, _args: &[&str], _services: &CommandServices) -> Vec<String> {
        vec![]
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
}

impl CommandHandler for InfrarustRootCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        _player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let sub_name = ctx.args.first().map(|s| s.to_lowercase());

            if let Some(player_id) = ctx.player_id
                && let Some(player) = self.services.player_registry.get_player_by_id(player_id)
            {
                let level = player.permission_level();
                match sub_name.as_deref() {
                    Some(name) => {
                        if !self
                            .services
                            .permission_service
                            .is_command_allowed(name, level)
                        {
                            let _ = player.send_message(ProxyMessage::error(NO_PERMISSION));
                            return;
                        }
                    }
                    None => {
                        if self
                            .services
                            .permission_service
                            .visible_subcommands(level)
                            .is_empty()
                        {
                            let _ = player.send_message(ProxyMessage::error(NO_PERMISSION));
                            return;
                        }
                    }
                }
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

    fn tab_complete(&self, partial_args: &[&str]) -> Vec<String> {
        match partial_args.len() {
            0 | 1 => {
                let prefix = partial_args.first().copied().unwrap_or("");
                self.subcommands
                    .keys()
                    .filter(|name| name.starts_with(prefix))
                    .cloned()
                    .collect()
            }
            _ => {
                let sub_name = partial_args[0].to_lowercase();
                if let Some(sub) = self.subcommands.get(&sub_name) {
                    sub.tab_complete(&partial_args[1..], &self.services)
                } else {
                    vec![]
                }
            }
        }
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
        config_service: Arc::new(ConfigServiceImpl::new(Arc::clone(
            &proxy_services.domain_router,
        ))),
        server_manager: proxy_services.server_manager.clone(),
        plugin_registry,
        command_manager: Arc::clone(&proxy_services.command_manager),
        permission_service: Arc::clone(&proxy_services.permission_service),
        start_time,
    });

    let root_cmd = InfrarustRootCommand::new(services);

    let mut all = HashSet::new();
    let mut admin_only = HashSet::new();
    for (name, sub) in &root_cmd.subcommands {
        all.insert(name.clone());
        if sub.required_level() >= PermissionLevel::Admin {
            admin_only.insert(name.clone());
        }
    }
    proxy_services
        .permission_service
        .register_subcommands(all, admin_only);

    command_manager.register_builtin(
        "infrarust",
        &["ir"],
        "Infrarust proxy commands",
        Box::new(root_cmd),
    );
}
