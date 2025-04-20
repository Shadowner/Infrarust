use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::shared_component::SharedComponent;
use tracing::debug;

pub struct KickCommand {
    shared: Arc<SharedComponent>,
}

impl KickCommand {
    pub fn new(shared: Arc<SharedComponent>) -> Self {
        Self { shared }
    }

    async fn find_and_kick(&self, username: &str, config_id: Option<&str>) -> String {
        debug!("Attempting to kick player: {}", username);
        let supervisor = self.shared.actor_supervisor();
        let actors = supervisor.get_all_actors().await;

        if actors.is_empty() {
            return fmt::warning("No players are currently connected.").to_string();
        }

        let mut matches = Vec::new();

        for (server_id, pairs) in actors {
            if let Some(config) = config_id {
                if server_id != config {
                    continue;
                }
            }

            for pair in pairs {
                if pair.username.eq_ignore_ascii_case(username) {
                    matches.push((server_id.clone(), pair));
                }
            }
        }

        if matches.is_empty() {
            if let Some(config) = config_id {
                return fmt::warning(&format!(
                    "No player with username '{}' found on server '{}'.",
                    fmt::entity(username),
                    fmt::entity(config)
                ))
                .to_string();
            } else {
                return fmt::warning(&format!(
                    "No player with username '{}' found on any server.",
                    fmt::entity(username)
                ))
                .to_string();
            }
        }

        if matches.len() == 1 || config_id.is_some() {
            let mut result = String::new();

            for (server_id, pair) in matches {
                pair.shutdown.store(true, Ordering::SeqCst);

                result.push_str(&fmt::success(&format!(
                    "Kicked player '{}' from server '{}'.",
                    fmt::entity(&pair.username),
                    fmt::entity(&server_id)
                )));
            }

            result
        } else {
            let mut result = fmt::warning(&format!(
                "Multiple players found with username '{}'. Please specify a server:\n",
                fmt::entity(username)
            ))
            .to_string();

            result.push_str(
                &fmt::secondary(&format!("Usage: kick {} <server-id>\n\n", username)).to_string(),
            );

            for (i, (server_id, pair)) in matches.iter().enumerate() {
                result.push_str(&format!(
                    "  {}. Server: {} {}\n",
                    i + 1,
                    fmt::entity(server_id),
                    fmt::id(&format!("(session: {})", pair.session_id))
                ));
            }

            result
        }
    }
}

impl Command for KickCommand {
    fn name(&self) -> &'static str {
        "kick"
    }

    fn description(&self) -> &'static str {
        "Kicks a player from the server. Usage: kick <username> [server-id]"
    }

    fn execute(&self, args: Vec<String>) -> CommandFuture {
        debug!("Executing kick command with args: {:?}", args);

        // Clone what we need for the async block
        let shared = self.shared.clone();

        Box::pin(async move {
            if args.is_empty() {
                return fmt::error("Usage: kick <username> [server-id]").to_string();
            }

            let username = &args[0];
            let config_id = args.get(1).map(|s| s.as_str());

            let kick_cmd = KickCommand { shared };
            kick_cmd.find_and_kick(username, config_id).await
        })
    }
}
