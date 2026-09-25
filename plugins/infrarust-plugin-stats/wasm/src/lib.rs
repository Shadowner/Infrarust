//! WASM adapter for the stats plugin. Wraps the same [`infrarust_plugin_stats`]
//! core as the native build, so behavior is identical across both.

#![forbid(unsafe_code)]

use infrarust_plugin_sdk::prelude::*;
use infrarust_plugin_stats as core;

#[derive(Default)]
struct StatsPlugin;

#[plugin(id = "stats", name = "Stats Plugin")]
impl Plugin for StatsPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            info!("[stats] {}", core::join_log(&event.profile.username));
        })?;
        ctx.on::<DisconnectEvent>(EventPriority::Normal, |event| {
            info!("[stats] {}", core::leave_log(&event.player.username));
        })?;
        ctx.command(core::COMMAND_NAME)
            .aliases(core::COMMAND_ALIASES.iter().copied())
            .description(core::COMMAND_DESCRIPTION)
            .handler(|invocation| {
                let reply = core::format_count(Players::count());
                let _ = invocation.reply(Component::text(reply));
            })
            .register()?;
        Ok(())
    }
}
