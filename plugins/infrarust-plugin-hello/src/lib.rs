//! Example plugin that logs connections and provides a `/hello` command.

use infrarust_api::prelude::*;

pub struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("hello", "Hello Plugin", "0.1.0")
            .author("Infrarust")
            .description("Example plugin: logs connections and provides /hello command")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, |event: &mut PostLoginEvent| {
                    tracing::info!("[HelloPlugin] {} joined the proxy!", event.profile.username);
                });

            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, |event: &mut DisconnectEvent| {
                    tracing::info!("[HelloPlugin] {} left the proxy", event.username);
                });

            ctx.command_manager().register(
                "hello",
                &["hi", "hey"],
                "Says hello to the player",
                Box::new(HelloCommand),
            );

            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, |event: &mut ChatMessageEvent| {
                    if event.message.contains("hello") {
                        tracing::info!(
                            "[HelloPlugin] Detected 'hello' in a chat message: {}",
                            event.message
                        );
                        tracing::info!("[HelloPlugin] Rejecting the message");
                        event.deny(Component::text("Test"));
                    }
                });
            let player_registry = ctx.player_registry_handle();
            ctx.scheduler().interval(
                std::time::Duration::from_secs(60),
                Box::new(move || {
                    tracing::info!("[HelloPlugin] 60 seconds have passed!");
                    player_registry.get_all_players().iter().for_each(|player| {
                        let _ = player.send_message(Component::text("Hello from the scheduler!"));
                    });
                }),
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
