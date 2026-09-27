use std::future::Future;
use std::pin::Pin;

use infrarust_api::services::config_service::{ConfigService, ServerConfig};

use crate::console::ConsoleServices;
use crate::console::commands::servers::{join_addresses, mode_name};
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{Block, CommandCategory, CommandOutput, Span, Table};

pub(crate) fn config_block(configs: &[ServerConfig]) -> Block {
    let count = configs.len();
    let meta = format!("{count} server{}", if count == 1 { "" } else { "s" });
    let block = Block::new("Configuration").meta(meta);
    if configs.is_empty() {
        return block;
    }

    let mut table = Table::new(&["Server", "Mode", "Addresses"]);
    for config in configs {
        table.row([
            Span::entity(config.id.as_str()),
            Span::plain(mode_name(config.proxy_mode)),
            Span::muted(join_addresses(&config.addresses)),
        ]);
    }
    block.table(table)
}

pub(crate) fn value_block(key: &str, value: String) -> Block {
    Block::new(key).line(value)
}

pub struct ReloadCommand;

impl ConsoleCommand for ReloadCommand {
    fn name(&self) -> &str {
        "reload"
    }

    fn description(&self) -> &str {
        "Reload configuration"
    }

    fn usage(&self) -> &str {
        "reload"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Config
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        _services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            CommandOutput::Note(
                "Configuration reloads on its own when the files change".to_string(),
            )
        })
    }
}

pub struct ConfigCommand;

impl ConsoleCommand for ConfigCommand {
    fn name(&self) -> &str {
        "config"
    }

    fn description(&self) -> &str {
        "Show configuration"
    }

    fn usage(&self) -> &str {
        "config [key]"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Config
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            match args.first() {
                Some(key) => match services.config_service.get_value(key) {
                    Some(value) => value_block(key, value).into(),
                    None => CommandOutput::error(format!("Configuration key '{key}' not found")),
                },
                None => config_block(&services.config_service.get_all_server_configs()).into(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use infrarust_api::services::config_service::ProxyMode;
    use infrarust_api::types::{ServerAddress, ServerId};

    use super::*;
    use crate::console::render::Renderer;

    fn render(block: Block) -> String {
        Renderer::new(false, None).render(&block.into())
    }

    #[test]
    fn the_overview_lists_every_server() {
        let configs = [
            ServerConfig::new(ServerId::new("lobby")).addresses(vec![
                ServerAddress {
                    host: "10.0.0.1".into(),
                    port: 25565,
                },
                ServerAddress {
                    host: "10.0.0.2".into(),
                    port: 25566,
                },
            ]),
            ServerConfig::new(ServerId::new("survival")).proxy_mode(ProxyMode::ClientOnly),
        ];
        assert_eq!(
            render(config_block(&configs)),
            "# Configuration - 2 servers\n\
             | SERVER     MODE          ADDRESSES\n\
             | lobby      passthrough   10.0.0.1:25565, 10.0.0.2:25566\n\
             | survival   client_only   -"
        );
    }

    #[test]
    fn an_empty_overview_has_only_the_header() {
        assert_eq!(render(config_block(&[])), "# Configuration - 0 servers");
    }

    #[test]
    fn a_key_shows_its_value() {
        assert_eq!(
            render(value_block("bind", "0.0.0.0:25565".into())),
            "# bind\n| 0.0.0.0:25565"
        );
    }
}
