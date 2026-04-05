use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::ServerId;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct SendSubcommand;

impl SubcommandHandler for SendSubcommand {
    fn name(&self) -> &str {
        "send"
    }

    fn description(&self) -> &str {
        "Send a player to a server"
    }

    fn required_level(&self) -> PermissionLevel {
        PermissionLevel::Admin
    }

    fn usage(&self) -> &str {
        "/ir send <player> <server>"
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player_id) = ctx.player_id else {
                return;
            };
            let Some(sender) = services.player_registry.get_player_by_id(player_id) else {
                return;
            };

            if args.len() < 2 {
                let _ =
                    sender.send_message(ProxyMessage::error("Usage: /ir send <player> <server>"));
                return;
            }

            let target_name = &args[0];
            let server_name = &args[1];

            let Some(target) = services.player_registry.get_player(target_name) else {
                let _ = sender.send_message(ProxyMessage::error(&format!(
                    "Player '{target_name}' is not online."
                )));
                return;
            };

            let target_server = ServerId::new(server_name);
            if services
                .config_service
                .get_server_config(&target_server)
                .is_none()
            {
                let _ = sender.send_message(ProxyMessage::error(&format!(
                    "Server '{server_name}' not found."
                )));
                return;
            }

            match target.switch_server(target_server).await {
                Ok(()) => {
                    let _ = sender.send_message(ProxyMessage::success(&format!(
                        "Sending {target_name} to '{server_name}'..."
                    )));
                }
                Err(e) => {
                    let _ = sender.send_message(ProxyMessage::error(&format!(
                        "Failed to send {target_name}: {e}"
                    )));
                }
            }
        })
    }

    fn tab_complete(&self, args: &[&str], services: &CommandServices) -> Vec<String> {
        match args.len() {
            0 | 1 => {
                let prefix = args.first().copied().unwrap_or("");
                services
                    .player_registry
                    .get_all_players()
                    .into_iter()
                    .map(|p| p.profile().username.clone())
                    .filter(|name| name.to_lowercase().starts_with(&prefix.to_lowercase()))
                    .collect()
            }
            2 => {
                let prefix = args[1];
                services
                    .config_service
                    .get_all_server_configs()
                    .into_iter()
                    .map(|cfg| cfg.id.as_str().to_string())
                    .filter(|name| name.starts_with(prefix))
                    .collect()
            }
            _ => vec![],
        }
    }
}
