use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::services::player_registry::PlayerRegistry;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct PluginsSubcommand;

impl SubcommandHandler for PluginsSubcommand {
    fn name(&self) -> &str {
        "plugins"
    }

    fn description(&self) -> &str {
        "List loaded plugins"
    }

    fn required_level(&self) -> PermissionLevel {
        PermissionLevel::Admin
    }

    fn usage(&self) -> &str {
        "/ir plugins"
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        _args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player_id) = ctx.player_id else {
                return;
            };
            let Some(player) = services.player_registry.get_player_by_id(player_id) else {
                return;
            };

            let plugins = services.plugin_registry.list_plugin_info();

            if plugins.is_empty() {
                let _ = player.send_message(ProxyMessage::info("No plugins loaded."));
                return;
            }

            let _ =
                player.send_message(ProxyMessage::info(&format!("Plugins ({}):", plugins.len())));

            for info in &plugins {
                let desc = info.description.as_deref().unwrap_or("No description");
                let _ = player.send_message(ProxyMessage::detail(&format!(
                    "  {} v{} - {}",
                    info.name, info.version, desc
                )));
            }
        })
    }
}
