//! Command implementations for the CLI.

mod configs;
mod kick;
mod players;

pub use configs::ConfigsCommand;
pub use kick::KickCommand;
pub use players::PlayersCommand;

use crate::cli::command::Command;
use crate::core::actors::supervisor::ActorSupervisor;
use crate::core::config::service::ConfigurationService;
use std::sync::Arc;

pub fn get_all_commands(
    supervisor: Option<Arc<ActorSupervisor>>,
    config_service: Option<Arc<ConfigurationService>>,
) -> Vec<Arc<dyn Command>> {
    let mut commands: Vec<Arc<dyn Command>> = vec![];

    if let Some(supervisor) = supervisor {
        commands.push(Arc::new(PlayersCommand::new(supervisor.clone())));
        commands.push(Arc::new(KickCommand::new(supervisor)));
    }

    if let Some(config_service) = config_service {
        commands.push(Arc::new(ConfigsCommand::new(config_service)));
    }

    commands
}
