//! Example plugin that logs connections and provides a `/hello` command.

use infrarust_api::prelude::*;

pub struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("hello", "Hello Plugin", "0.1.0")
            .author("Infrarust")
            .description("Example plugin: logs connections and provides /hello command")
    }

    fn on_enable<'a>(&'a self, ctx: &'a dyn PluginContext) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            ctx.event_bus().subscribe(
                EventPriority::NORMAL,
                |event: &mut PostLoginEvent| {
                    tracing::info!("[HelloPlugin] {} joined the proxy!", event.profile.username);
                },
            );

            ctx.event_bus().subscribe(
                EventPriority::NORMAL,
                |event: &mut DisconnectEvent| {
                    tracing::info!("[HelloPlugin] {} left the proxy", event.username);
                },
            );

            ctx.command_manager().register(
                "hello",
                &["hi", "hey"],
                "Says hello to the player",
                Box::new(HelloCommand),
            );

            tracing::info!("[HelloPlugin] Enabled successfully");
            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async {
            tracing::info!("[HelloPlugin] Disabled");
            Ok(())
        })
    }
}

struct HelloCommand;

impl CommandHandler for HelloCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(id) = ctx.player_id
                && let Some(player) = player_registry.get_player_by_id(id)
            {
                let _ = player.send_message(
                    Component::text("Hello from Infrarust! ")
                        .color("gold")
                        .bold()
                        .append(Component::text("Welcome to the proxy.").color("gray")),
                );
            }
        })
    }
}
