//! WASM adapter for the stats plugin. Wraps the same [`infrarust_plugin_stats`]
//! core as the native build, so behavior is identical across both.

use infrarust_plugin_sdk::prelude::*;
use infrarust_plugin_stats as core;

#[derive(Default)]
struct StatsPlugin;

#[plugin(id = "stats", name = "Stats Plugin")]
impl Plugin for StatsPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            info!("[stats] {}", core::join_log(&event.profile.username));
        });
        ctx.on::<DisconnectEvent>(EventPriority::Normal, |event| {
            info!("[stats] {}", core::leave_log(&event.username));
        });
        ctx.command(core::COMMAND_NAME, |invocation| {
            let reply = core::format_count(Players.online_count());
            if let Some(id) = invocation.player
                && let Some(player) = Players.get_by_id(id)
            {
                let _ = player.send_message(&Component::text(&reply).into_json());
            }
        })
        .aliases(core::COMMAND_ALIASES.iter().copied())
        .description(core::COMMAND_DESCRIPTION)
        .register();
        Ok(())
    }
}
