use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::ServerId;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct ServerSubcommand;

impl SubcommandHandler for ServerSubcommand {
    fn name(&self) -> &str {
        "server"
    }

    fn description(&self) -> &str {
        "Show or switch current server"
    }

    fn usage(&self) -> &str {
        "/ir server [name]"
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
            let Some(player) = services.player_registry.get_player_by_id(player_id) else {
                return;
            };

            match args.first() {
                None => match player.current_server() {
                    Some(server) => {
                        let _ = player.send_message(ProxyMessage::info(&format!(
                            "You are connected to: {}",
                            server.as_str()
                        )));
                    }
                    None => {
                        let _ = player.send_message(ProxyMessage::info(
                            "You are not connected to any server.",
                        ));
                    }
                },
                Some(name) => {
                    let target = ServerId::new(name);

                    if services.config_service.get_server_config(&target).is_none() {
                        let _ = player.send_message(ProxyMessage::error(&format!(
                            "Server '{name}' not found."
                        )));
                        return;
                    }

                    match player.switch_server(target).await {
                        Ok(()) => {
                            let _ = player.send_message(ProxyMessage::success(&format!(
                                "Switching to server '{name}'..."
                            )));
                        }
                        Err(e) => {
                            let _ = player.send_message(ProxyMessage::error(&format!(
                                "Failed to switch: {e}"
                            )));
                        }
                    }
                }
            }
        })
    }

    fn tab_complete(&self, args: &[&str], services: &CommandServices) -> Vec<String> {
        if args.len() <= 1 {
            let prefix = args.first().copied().unwrap_or("");
            services
                .config_service
                .get_all_server_configs()
                .into_iter()
                .map(|cfg| cfg.id.as_str().to_string())
                .filter(|name| name.starts_with(prefix))
                .collect()
        } else {
            vec![]
        }
    }
}
