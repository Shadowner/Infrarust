//! Command implementations for the CLI.

mod ban;
mod banlist;
mod configs;
mod kick;
mod players;
mod unban;

pub use ban::BanCommand;
pub use banlist::BanListCommand;
pub use configs::ConfigsCommand;
pub use kick::KickCommand;
pub use players::PlayersCommand;
pub use unban::UnbanCommand;

use crate::cli::command::Command;
use crate::core::shared_component::SharedComponent;
use std::sync::Arc;

pub fn get_all_commands(shared_component: Option<Arc<SharedComponent>>) -> Vec<Arc<dyn Command>> {
    let mut commands: Vec<Arc<dyn Command>> = vec![];

    if let Some(shared) = shared_component {
        commands.push(Arc::new(PlayersCommand::new(shared.clone())));
        commands.push(Arc::new(KickCommand::new(shared.clone())));
        commands.push(Arc::new(ConfigsCommand::new(shared.clone())));
        commands.push(Arc::new(BanCommand::new(shared.clone())));
        commands.push(Arc::new(UnbanCommand::new(shared.clone())));
        commands.push(Arc::new(BanListCommand::new(shared.clone())));
    }

    commands
}
