pub mod bans;
pub mod config;
pub mod permissions;
pub mod players;
pub mod plugins;
pub mod servers;
pub mod system;

use std::time::Duration;

use comfy_table::{Cell, Table};

use super::dispatcher::{CommandDispatcher, ConsoleCommand};
use super::output::{CommandOutput, OutputRenderer};
use super::parser::parse_duration_arg;

pub(crate) fn usage(command: &dyn ConsoleCommand) -> CommandOutput {
    CommandOutput::Error(format!("Usage: {}", command.usage()))
}

pub(crate) mod args {
    use super::{Duration, parse_duration_arg};

    pub fn rest(args: &[&str]) -> Option<String> {
        (!args.is_empty()).then(|| args.join(" "))
    }

    pub fn duration_and_reason(args: &[&str]) -> (Option<Duration>, Option<String>) {
        match args.split_first() {
            Some((head, tail)) => match parse_duration_arg(head) {
                Ok(duration) => (duration, rest(tail)),
                Err(_) => (None, rest(args)),
            },
            None => (None, None),
        }
    }
}

pub(crate) struct TableBuilder {
    table: Table,
    rows: usize,
}

pub(crate) fn table(headers: &[&str]) -> TableBuilder {
    let mut table = OutputRenderer::new().create_table();
    table.set_header(headers.to_vec());
    TableBuilder { table, rows: 0 }
}

impl TableBuilder {
    pub fn row<C: Into<Cell>>(&mut self, cells: impl IntoIterator<Item = C>) -> &mut Self {
        self.table.add_row(cells);
        self.rows += 1;
        self
    }

    pub fn finish(self, unit: &str) -> CommandOutput {
        CommandOutput::Table {
            table: self.table,
            footer: Some(format!(" {} {unit}(s)", self.rows)),
        }
    }
}

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

    dispatcher.register(Box::new(permissions::OpCommand));
    dispatcher.register(Box::new(permissions::DeopCommand));
    dispatcher.register(Box::new(permissions::OpListCommand));

    let help = system::HelpCommand::from_commands(dispatcher.command_info());
    dispatcher.register(Box::new(help));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn duration_and_reason_split_on_a_leading_duration() {
        assert_eq!(args::duration_and_reason(&[]), (None, None));
        assert_eq!(
            args::duration_and_reason(&["griefing", "a", "lot"]),
            (None, Some("griefing a lot".to_string()))
        );
        assert_eq!(
            args::duration_and_reason(&["permanent", "bye"]),
            (None, Some("bye".to_string()))
        );
        let (duration, reason) = args::duration_and_reason(&["1h", "cool", "off"]);
        assert_eq!(duration, Some(Duration::from_secs(3600)));
        assert_eq!(reason.as_deref(), Some("cool off"));
        assert_eq!(
            args::duration_and_reason(&["1h"]),
            (Some(Duration::from_secs(3600)), None)
        );
    }

    #[test]
    fn table_footer_counts_the_rows() {
        let mut builder = table(&["A", "B"]);
        builder.row([Cell::new("1"), Cell::new("2")]);
        builder.row([Cell::new("3"), Cell::new("4")]);
        match builder.finish("row") {
            CommandOutput::Table { table, footer } => {
                assert_eq!(footer.as_deref(), Some(" 2 row(s)"));
                assert_eq!(table.row_count(), 2);
            }
            _ => panic!("expected a table"),
        }
    }
}
