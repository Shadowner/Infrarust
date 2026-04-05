use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, ServerId};

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct BroadcastSubcommand;

impl SubcommandHandler for BroadcastSubcommand {
    fn name(&self) -> &str {
        "broadcast"
    }

    fn description(&self) -> &str {
        "Broadcast a message to all players"
    }

    fn required_level(&self) -> PermissionLevel {
        PermissionLevel::Admin
    }

    fn usage(&self) -> &str {
        "/ir broadcast <message> [--server <name>]"
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

            if args.is_empty() {
                let _ = sender.send_message(ProxyMessage::error(
                    "Usage: /ir broadcast <message> [--server <name>]",
                ));
                return;
            }

            let mut message_parts: Vec<&str> = Vec::new();
            let mut target_server: Option<String> = None;
            let mut skip_next = false;

            for (i, arg) in args.iter().enumerate() {
                if skip_next {
                    skip_next = false;
                    continue;
                }
                if arg == "--server" {
                    if let Some(server) = args.get(i + 1) {
                        target_server = Some(server.clone());
                        skip_next = true;
                    }
                } else {
                    message_parts.push(arg);
                }
            }

            if message_parts.is_empty() {
                let _ = sender.send_message(ProxyMessage::error("No message provided."));
                return;
            }

            let raw_message = message_parts.join(" ");
            let message = Component::from_legacy(&raw_message);

            let recipients = match &target_server {
                Some(server_name) => {
                    let server_id = ServerId::new(server_name);
                    if services
                        .config_service
                        .get_server_config(&server_id)
                        .is_none()
                    {
                        let _ = sender.send_message(ProxyMessage::error(&format!(
                            "Server '{server_name}' not found."
                        )));
                        return;
                    }
                    services.player_registry.get_players_on_server(&server_id)
                }
                None => services.player_registry.get_all_players(),
            };

            let count = recipients.len();
            for player in &recipients {
                let _ = player.send_message(message.clone());
            }

            let scope = target_server
                .as_deref()
                .map(|s| format!(" on '{s}'"))
                .unwrap_or_default();

            let _ = sender.send_message(ProxyMessage::success(&format!(
                "Broadcast sent to {count} player{}{scope}.",
                if count == 1 { "" } else { "s" }
            )));
        })
    }

    fn tab_complete(&self, args: &[&str], services: &CommandServices) -> Vec<String> {
        let last = args.last().copied().unwrap_or("");
        let prev = if args.len() >= 2 {
            args[args.len() - 2]
        } else {
            ""
        };

        if prev == "--server" {
            services
                .config_service
                .get_all_server_configs()
                .into_iter()
                .map(|cfg| cfg.id.as_str().to_string())
                .filter(|name| name.starts_with(last))
                .collect()
        } else if "--server".starts_with(last) {
            vec!["--server".to_string()]
        } else {
            vec![]
        }
    }
}
