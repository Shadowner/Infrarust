pub mod bans;
pub mod config;
pub mod players;
pub mod plugins;
pub mod servers;
pub mod system;

use super::dispatcher::CommandDispatcher;

pub fn register_all(dispatcher: &mut CommandDispatcher) {
    dispatcher.register(Box::new(players::ListPlayersCommand));
    dispatcher.register(Box::new(players::FindPlayerCommand));
    dispatcher.register(Box::new(players::KickCommand));
    dispatcher.register(Box::new(players::KickIpCommand));
    dispatcher.register(Box::new(players::SendCommand));
    dispatcher.register(Box::new(players::SendAllCommand));
    dispatcher.register(Box::new(players::MsgCommand));
    dispatcher.register(Box::new(players::BroadcastCommand));

    dispatcher.register(Box::new(bans::BanCommand));
    dispatcher.register(Box::new(bans::BanIpCommand));
    dispatcher.register(Box::new(bans::UnbanCommand));
    dispatcher.register(Box::new(bans::UnbanIpCommand));
    dispatcher.register(Box::new(bans::BanListCommand));
    dispatcher.register(Box::new(bans::BanInfoCommand));

    dispatcher.register(Box::new(servers::ServersCommand));
    dispatcher.register(Box::new(servers::ServerCommand));
    dispatcher.register(Box::new(servers::StartServerCommand));
    dispatcher.register(Box::new(servers::StopServerCommand));

    dispatcher.register(Box::new(config::ReloadCommand));
    dispatcher.register(Box::new(config::ConfigCommand));

    dispatcher.register(Box::new(plugins::PluginsCommand));
    dispatcher.register(Box::new(plugins::PluginCommand));

    dispatcher.register(Box::new(system::VersionCommand));
    dispatcher.register(Box::new(system::StatusCommand));
    dispatcher.register(Box::new(system::StopCommand));
    dispatcher.register(Box::new(system::ClearCommand));
    dispatcher.register(Box::new(system::GcCommand));

    let help = system::HelpCommand::from_commands(dispatcher.command_info());
    dispatcher.register(Box::new(help));
}
