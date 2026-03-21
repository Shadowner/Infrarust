//! Configuration commands: reload, config.

use std::future::Future;
use std::pin::Pin;

use infrarust_api::services::config_service::ConfigService;

use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{CommandCategory, CommandOutput, OutputLine};
use crate::console::ConsoleServices;

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
            CommandOutput::Success(
                "Configuration is auto-reloaded via file watcher. Modify the config files and changes will apply automatically.".to_string(),
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
            if let Some(key) = args.first() {
                return match services.config_service.get_value(key) {
                    Some(value) => CommandOutput::Lines(vec![OutputLine::Info(format!(
                        "  {key} = {value}"
                    ))]),
                    None => CommandOutput::Error(format!("Configuration key '{key}' not found")),
                };
            }

            let configs = services.config_service.get_all_server_configs();

            let mut lines = vec![
                OutputLine::Info(format!(
                    "  Servers configured: {}",
                    configs.len()
                )),
            ];

            for cfg in &configs {
                let addresses: Vec<String> = cfg
                    .addresses
                    .iter()
                    .map(|a| format!("{}:{}", a.host, a.port))
                    .collect();
                lines.push(OutputLine::Info(format!(
                    "    {} -> {} ({:?})",
                    cfg.id.as_str(),
                    addresses.join(", "),
                    cfg.proxy_mode,
                )));
            }

            CommandOutput::Lines(lines)
        })
    }
}
