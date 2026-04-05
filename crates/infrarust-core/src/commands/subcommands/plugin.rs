use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::services::player_registry::PlayerRegistry;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct PluginSubcommand;

impl SubcommandHandler for PluginSubcommand {
    fn name(&self) -> &str {
        "plugin"
    }

    fn description(&self) -> &str {
        "Run a plugin command by namespace"
    }

    fn required_level(&self) -> PermissionLevel {
        PermissionLevel::Admin
    }

    fn usage(&self) -> &str {
        "/ir plugin <plugin_id> <command> [args...]"
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player_id) = ctx.player_id else {
                return;
            };
            let Some(player) = services.player_registry.get_player_by_id(player_id) else {
                return;
            };

            match args.len() {
                0 => {
                    let plugins = services.plugin_registry.list_plugin_info();
                    if plugins.is_empty() {
                        let _ = player.send_message(ProxyMessage::info("No plugins loaded."));
                        return;
                    }
                    let _ = player
                        .send_message(ProxyMessage::info(&format!("Plugins ({}):", plugins.len())));
                    for info in &plugins {
                        let desc = info.description.as_deref().unwrap_or("No description");
                        let _ = player.send_message(ProxyMessage::detail(&format!(
                            "  {} v{} - {}",
                            info.name, info.version, desc
                        )));
                    }
                }
                1 => {
                    let plugin_id = &args[0];
                    let cmds = services.command_manager.commands_for_plugin(plugin_id);
                    if cmds.is_empty() {
                        let _ = player.send_message(ProxyMessage::error(&format!(
                            "Plugin '{}' not found or has no commands.",
                            plugin_id
                        )));
                        return;
                    }
                    let _ = player.send_message(ProxyMessage::info(&format!(
                        "Commands for plugin '{}':",
                        plugin_id
                    )));
                    for (name, desc) in &cmds {
                        let _ = player
                            .send_message(ProxyMessage::detail(&format!("  /{} - {}", name, desc)));
                    }
                }
                _ => {
                    let plugin_id = &args[0];
                    let command_name = &args[1];
                    let handler = services
                        .command_manager
                        .find_plugin_command(plugin_id, command_name);

                    match handler {
                        Some(handler) => {
                            let sub_args: Vec<String> = args.iter().skip(2).cloned().collect();
                            let sub_ctx = CommandContext {
                                player_id: ctx.player_id,
                                args: sub_args,
                                raw: ctx.raw.clone(),
                            };
                            handler
                                .execute(sub_ctx, services.player_registry.as_ref())
                                .await;
                        }
                        None => {
                            let _ = player.send_message(ProxyMessage::error(&format!(
                                "Command '{}' not found for plugin '{}'.",
                                command_name, plugin_id
                            )));
                        }
                    }
                }
            }
        })
    }

    fn tab_complete(&self, args: &[&str], services: &CommandServices) -> Vec<String> {
        match args.len() {
            0 | 1 => {
                let prefix = args.first().copied().unwrap_or("");
                let plugins = services.plugin_registry.list_plugin_info();
                plugins
                    .into_iter()
                    .map(|p| p.id)
                    .filter(|id| id.starts_with(prefix))
                    .collect()
            }
            2 => {
                let plugin_id = args[0];
                let prefix = args[1];
                services
                    .command_manager
                    .commands_for_plugin(plugin_id)
                    .into_iter()
                    .map(|(name, _)| name)
                    .filter(|name| name.starts_with(prefix))
                    .collect()
            }
            _ => {
                let plugin_id = args[0];
                let command_name = args[1];
                if let Some(handler) = services
                    .command_manager
                    .find_plugin_command(plugin_id, command_name)
                {
                    handler.tab_complete(&args[2..])
                } else {
                    vec![]
                }
            }
        }
    }
}
