//! Native adapter for the stats plugin. Wraps [`infrarust_plugin_stats`] behind
//! the native `Plugin` trait; the WASM adapter wraps the same core.

use infrarust_api::prelude::*;
use infrarust_plugin_stats as core;

#[derive(Default)]
pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("stats", "Stats Plugin", env!("CARGO_PKG_VERSION"))
            .author("Infrarust")
            .description("Logs joins/leaves and serves a /count command")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, |event: &mut PostLoginEvent| {
                    tracing::info!("[stats] {}", core::join_log(&event.profile.username));
                });
            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, |event: &mut DisconnectEvent| {
                    tracing::info!("[stats] {}", core::leave_log(&event.username));
                });
            ctx.command_manager().register(
                core::COMMAND_NAME,
                core::COMMAND_ALIASES,
                core::COMMAND_DESCRIPTION,
                Box::new(CountCommand),
            );
            Ok(())
        })
    }
}

struct CountCommand;

impl CommandHandler for CountCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        players: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let online = u32::try_from(players.online_count()).unwrap_or(u32::MAX);
            if let Some(id) = ctx.player_id
                && let Some(player) = players.get_player_by_id(id)
            {
                let _ = player.send_message(Component::text(core::format_count(online)));
            }
        })
    }
}
