//! Example plugin that logs connections, provides a `/hello` command,
//! and demonstrates the Limbo engine with a `/limbo` + `/success` flow.

use infrarust_api::prelude::*;

pub struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("hello", "Hello Plugin", "0.1.0")
            .author("Infrarust")
            .description("Example plugin: logs connections, /hello command, limbo test gate")
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

            ctx.command_manager().register(
                "hello",
                &["hi", "hey"],
                "Says hello to the player",
                Box::new(HelloCommand),
            );

            ctx.command_manager().register(
                "limbo",
                &[],
                "Sends you to the limbo test gate",
                Box::new(LimboCommand),
            );

            ctx.register_limbo_handler(Box::new(TestGateHandler));

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

            tracing::info!("[HelloPlugin] Enabled successfully (limbo handler 'test-gate' registered)");
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

struct LimboCommand;

impl CommandHandler for LimboCommand {
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
                    Component::text("Sending you to limbo...").color("yellow"),
                );
                // "$limbo" is a sentinel that the connection handler recognizes
                // to enter limbo mode using the current server's limbo_handlers.
                let _ = player.switch_server(ServerId::new("$limbo")).await;
            }
        })
    }
}

/// A test limbo handler that holds the player in a void world until they
/// type `/success`. Demonstrates send_message, send_title, on_command,
/// and the Hold → complete(Accept) flow.
struct TestGateHandler;

impl LimboHandler for TestGateHandler {
    fn name(&self) -> &str {
        "test-gate"
    }

    fn on_player_enter(&self, session: &dyn LimboSession) -> BoxFuture<'_, HandlerResult> {
        let username = session.profile().username.clone();
        tracing::info!("[TestGate] {username} entered limbo");

        let _ = session.send_title(TitleData::new(
            Component::text("Limbo").color("gold").bold(),
            Component::text("Type /success to continue").color("gray"),
        ));

        let _ = session.send_message(
            Component::text("You are in the limbo test gate. ").color("yellow")
                .append(Component::text("Type ").color("gray"))
                .append(Component::text("/success").color("green").bold())
                .append(Component::text(" to proceed to the server.").color("gray")),
        );

        Box::pin(async { HandlerResult::Hold })
    }

    fn on_command(
        &self,
        session: &dyn LimboSession,
        command: &str,
        _args: &[&str],
    ) -> BoxFuture<'_, ()> {
        let player_id = session.player_id();

        if command == "success" {
            tracing::info!("[TestGate] Player {player_id:?} typed /success, releasing from limbo");
            let _ = session.send_message(
                Component::text("Redirecting to server...").color("green"),
            );
            // complete() is synchronous — call it before the async block
            session.complete(HandlerResult::Accept);
        } else {
            let _ = session.send_message(
                Component::text(&format!("Unknown command: /{command}")).color("red"),
            );
        }

        Box::pin(async {})
    }

    fn on_chat(&self, session: &dyn LimboSession, message: &str) -> BoxFuture<'_, ()> {
        let msg = message.to_string();
        let _ = session.send_message(
            Component::text("You said: ").color("gray")
                .append(Component::text(&msg).color("white")),
        );
        Box::pin(async {})
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tracing::info!("[TestGate] Player {player_id:?} disconnected from limbo");
        })
    }
}
