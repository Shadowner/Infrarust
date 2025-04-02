use std::collections::HashMap;
use std::sync::Arc;

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::actors::supervisor::ActorSupervisor;
use tracing::debug;

pub struct PlayersCommand {
    supervisor: Arc<ActorSupervisor>,
}

impl PlayersCommand {
    pub fn new(supervisor: Arc<ActorSupervisor>) -> Self {
        Self { supervisor }
    }

    async fn format_player_list(&self) -> String {
        let mut result = String::new();
        let actors = self.supervisor.get_all_actors().await;

        if actors.is_empty() {
            return fmt::warning("No players connected.").to_string();
        }

        let mut players_by_config: HashMap<String, Vec<(String, String, uuid::Uuid)>> =
            HashMap::new();

        for (config_id, pairs) in actors {
            for pair in pairs {
                if !pair.username.is_empty()
                    && !pair.shutdown.load(std::sync::atomic::Ordering::SeqCst)
                {
                    let addr = match pair.client.get_peer_addr().await {
                        Ok(addr) => addr.to_string(),
                        Err(_) => "unknown".to_string(),
                    };

                    players_by_config
                        .entry(config_id.clone())
                        .or_default()
                        .push((pair.username.clone(), addr, pair.session_id));
                }
            }
        }

        let tota_players = players_by_config.values().map(|v| v.len()).sum::<usize>();
        result.push_str(&format!("{}\n\n", fmt::header("Connected Players")));

        for (config_id, players) in players_by_config {
            result.push_str(&format!(
                "{} {} {}\n",
                fmt::sub_header("Server"),
                fmt::entity(&config_id),
                fmt::secondary(&format!("({} players)", players.len()))
            ));

            for (i, (username, addr, session_id)) in players.iter().enumerate() {
                result.push_str(&format!(
                    "  {}. {} - {} {}\n",
                    i + 1,
                    fmt::entity(username),
                    fmt::secondary(addr),
                    fmt::id(&format!("(session: {})", session_id))
                ));
            }
            result.push('\n');
        }
        result.push_str(&format!(
            "{} \n",
            fmt::header(&format!("Total Players ({})", &tota_players.to_string()))
        ));
        result
    }
}

impl Command for PlayersCommand {
    fn name(&self) -> &'static str {
        "list"
    }

    fn description(&self) -> &'static str {
        "Lists all connected players by server"
    }

    fn execute(&self, _args: Vec<String>) -> CommandFuture {
        debug!("Executing players command");
        let supervisor = self.supervisor.clone();

        Box::pin(async move {
            let players_cmd = PlayersCommand { supervisor };
            players_cmd.format_player_list().await
        })
    }
}
