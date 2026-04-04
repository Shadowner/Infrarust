use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::services::player_registry::PlayerRegistry;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct VersionSubcommand;

impl SubcommandHandler for VersionSubcommand {
    fn name(&self) -> &str {
        "version"
    }

    fn description(&self) -> &str {
        "Show proxy version and status"
    }

    fn usage(&self) -> &str {
        "/ir version"
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

            let version = env!("CARGO_PKG_VERSION");
            let players = services.player_registry.online_count();
            let uptime = format_uptime(services.start_time.elapsed());

            let _ = player.send_message(ProxyMessage::info(&format!("Infrarust v{version}")));
            let _ = player.send_message(ProxyMessage::detail(&format!(
                "  Players online: {players}"
            )));
            let _ = player.send_message(ProxyMessage::detail(&format!("  Uptime: {uptime}")));
        })
    }
}

fn format_uptime(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;

    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}
