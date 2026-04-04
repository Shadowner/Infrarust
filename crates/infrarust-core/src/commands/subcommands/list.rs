use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct ListSubcommand;

impl SubcommandHandler for ListSubcommand {
    fn name(&self) -> &str {
        "list"
    }

    fn description(&self) -> &str {
        "List all servers"
    }

    fn usage(&self) -> &str {
        "/ir list"
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

            let configs = services.config_service.get_all_server_configs();

            if configs.is_empty() {
                let _ = player.send_message(ProxyMessage::info("No servers configured."));
                return;
            }

            let managed_states: std::collections::HashMap<
                String,
                infrarust_server_manager::ServerState,
            > = services
                .server_manager
                .as_ref()
                .map(|sm| sm.get_all_managed().into_iter().collect())
                .unwrap_or_default();

            let _ =
                player.send_message(ProxyMessage::info(&format!("Servers ({}):", configs.len())));

            for cfg in &configs {
                let id = cfg.id.as_str();
                let players = services.player_registry.online_count_on(&cfg.id);

                let state_str = managed_states
                    .get(id)
                    .map(format_state_indicator)
                    .unwrap_or_else(|| "-".to_string());

                let _ = player.send_message(ProxyMessage::detail(&format!(
                    "  {state_str} {id}  ({players} player{})",
                    if players == 1 { "" } else { "s" }
                )));
            }
        })
    }
}

fn format_state_indicator(state: &infrarust_server_manager::ServerState) -> String {
    use infrarust_server_manager::ServerState;
    match state {
        ServerState::Online => "\u{25CF}".to_string(),   // ●
        ServerState::Sleeping => "\u{25CB}".to_string(), // ○
        ServerState::Starting => "\u{25CF}".to_string(), // ●
        ServerState::Stopping => "\u{25CF}".to_string(), // ●
        ServerState::Crashed => "\u{2717}".to_string(),  // ✗
        _ => "?".to_string(),
    }
}
