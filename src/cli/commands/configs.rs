use std::sync::Arc;

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::config::service::ConfigurationService;
use tracing::debug;

pub struct ConfigsCommand {
    config_service: Arc<ConfigurationService>,
}

impl ConfigsCommand {
    pub fn new(config_service: Arc<ConfigurationService>) -> Self {
        Self { config_service }
    }

    async fn list_configs(&self) -> String {
        debug!("Listing server configurations");

        let configs = self.config_service.get_all_configurations().await;

        if configs.is_empty() {
            return fmt::warning("No server configurations found.").to_string();
        }

        let mut result = format!(
            "{}\n\n",
            fmt::header(&format!("Server Configurations ({} total)", configs.len()))
        );

        let mut config_list: Vec<_> = configs.iter().collect();
        config_list.sort_by(|a, b| a.0.cmp(b.0));

        for (config_id, config) in config_list {
            result.push_str(&format!("{}\n", fmt::sub_header(config_id)));

            result.push_str(&format!("  {}: ", fmt::label("Domains")));
            if config.domains.is_empty() {
                result.push_str(&fmt::secondary("none\n").to_string());
            } else {
                let domains: Vec<String> = config.domains.iter().map(|d| fmt::entity(d)).collect();
                result.push_str(&format!("{}\n", domains.join(", ")));
            }

            result.push_str(&format!("  {}: ", fmt::label("Addresses")));
            if config.addresses.is_empty() {
                result.push_str(&fmt::secondary("none\n").to_string());
            } else {
                let addrs: Vec<String> =
                    config.addresses.iter().map(|a| fmt::secondary(a)).collect();
                result.push_str(&format!("{}\n", addrs.join(", ")));
            }

            let mode_str = config
                .proxy_mode
                .as_ref()
                .map_or("Default", |mode| match mode {
                    crate::proxy_modes::ProxyModeEnum::Passthrough => "Passthrough",
                    crate::proxy_modes::ProxyModeEnum::Offline => "Offline",
                    crate::proxy_modes::ProxyModeEnum::Status => "Status Only",
                    crate::proxy_modes::ProxyModeEnum::ClientOnly => "Client Only",
                    crate::proxy_modes::ProxyModeEnum::ServerOnly => "Server Only",
                });

            result.push_str(&format!(
                "  {}: {}\n",
                fmt::label("Proxy Mode"),
                fmt::entity(mode_str)
            ));

            let proxy_str = config
                .send_proxy_protocol
                .map_or("Disabled", |v| if v { "Enabled" } else { "Disabled" });
            result.push_str(&format!(
                "  {}: {}\n",
                fmt::label("Proxy Protocol"),
                fmt::entity(proxy_str)
            ));

            result.push('\n');
        }

        result
    }
}

impl Command for ConfigsCommand {
    fn name(&self) -> &'static str {
        "configs"
    }

    fn description(&self) -> &'static str {
        "Lists all server configurations"
    }

    fn execute(&self, _args: Vec<String>) -> CommandFuture {
        debug!("Executing configs command");
        let config_service = self.config_service.clone();

        Box::pin(async move {
            let cmd = ConfigsCommand { config_service };
            cmd.list_configs().await
        })
    }
}
