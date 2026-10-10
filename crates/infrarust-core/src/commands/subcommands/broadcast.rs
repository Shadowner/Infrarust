use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::{CommandContext, CommandSource};
use infrarust_api::event::BoxFuture;

use crate::commands::actions::broadcast;
use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct BroadcastSubcommand;

impl SubcommandHandler for BroadcastSubcommand {
    fn name(&self) -> &str {
        "broadcast"
    }

    fn description(&self) -> &str {
        "Broadcast a message to all players"
    }

    fn admin_only(&self) -> bool {
        true
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
            let sender = &ctx.source;

            if args.is_empty() {
                sender.send_message(ProxyMessage::error(
                    "Usage: /ir broadcast <message> [--server <name>]",
                ));
                return;
            }

            let mut message_parts: Vec<&str> = Vec::new();
            let mut target_server: Option<&str> = None;
            let mut skip_next = false;

            for (i, arg) in args.iter().enumerate() {
                if skip_next {
                    skip_next = false;
                    continue;
                }
                if arg == "--server" {
                    if let Some(server) = args.get(i + 1) {
                        target_server = Some(server);
                        skip_next = true;
                    }
                } else {
                    message_parts.push(arg);
                }
            }

            let message = match broadcast(
                &*services.player_registry,
                &*services.config_service,
                target_server,
                &message_parts.join(" "),
            ) {
                Ok(sent) => {
                    let scope = sent
                        .scope
                        .as_ref()
                        .map(|server| format!(" on '{}'", server.as_str()))
                        .unwrap_or_default();
                    ProxyMessage::success(&format!(
                        "Broadcast sent to {} player{}{scope}.",
                        sent.recipients,
                        if sent.recipients == 1 { "" } else { "s" }
                    ))
                }
                Err(error) => ProxyMessage::error(&error.to_string()),
            };
            sender.send_message(message);
        })
    }

    fn tab_complete<'a>(
        &'a self,
        args: &'a [String],
        _source: &'a CommandSource,
        services: &'a CommandServices,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            let last = args.last().map(String::as_str).unwrap_or("");
            let prev = if args.len() >= 2 {
                args[args.len() - 2].as_str()
            } else {
                ""
            };

            if prev == "--server" {
                services.complete_server_names(last)
            } else if "--server".starts_with(last) {
                vec!["--server".to_string()]
            } else {
                vec![]
            }
        })
    }
}
