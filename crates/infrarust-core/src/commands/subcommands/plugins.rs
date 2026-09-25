use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct PluginsSubcommand;

impl SubcommandHandler for PluginsSubcommand {
    fn name(&self) -> &str {
        "plugins"
    }

    fn description(&self) -> &str {
        "List loaded plugins"
    }

    fn admin_only(&self) -> bool {
        true
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
            let player = &ctx.source;

            let plugins = services.plugin_registry.list_plugin_info();

            if plugins.is_empty() {
                player.send_message(ProxyMessage::info("No plugins loaded."));
                return;
            }

            player.send_message(ProxyMessage::info(&format!("Plugins ({}):", plugins.len())));

            for info in &plugins {
                let desc = info.description.as_deref().unwrap_or("No description");
                player.send_message(ProxyMessage::detail(&format!(
                    "  {} v{} - {}",
                    info.name, info.version, desc
                )));
            }
        })
    }
}
