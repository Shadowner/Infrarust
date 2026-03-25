//! Plugin commands: plugins, plugin.

use std::future::Future;
use std::pin::Pin;

use comfy_table::Cell;

use crate::console::ConsoleServices;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{CommandCategory, CommandOutput, OutputLine};
use crate::plugin::PluginState;

pub struct PluginsCommand;

impl ConsoleCommand for PluginsCommand {
    fn name(&self) -> &str {
        "plugins"
    }

    fn aliases(&self) -> &[&str] {
        &["pl"]
    }

    fn description(&self) -> &str {
        "List loaded plugins"
    }

    fn usage(&self) -> &str {
        "plugins"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Plugins
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let pm = services.plugin_manager.read().await;
            let plugins = pm.list_plugins();

            if plugins.is_empty() {
                return CommandOutput::Success("No plugins loaded".to_string());
            }

            let renderer = crate::console::output::OutputRenderer::new();
            let mut table = renderer.create_table();
            table.set_header(vec!["ID", "Name", "Version", "State"]);

            for meta in &plugins {
                let state = pm
                    .plugin_state(&meta.id)
                    .map(format_plugin_state)
                    .unwrap_or_else(|| "Unknown".to_string());

                table.add_row(vec![
                    Cell::new(&meta.id),
                    Cell::new(&meta.name),
                    Cell::new(&meta.version),
                    Cell::new(state),
                ]);
            }

            CommandOutput::Table {
                table,
                footer: Some(format!(" {} plugin(s)", plugins.len())),
            }
        })
    }
}

pub struct PluginCommand;

impl ConsoleCommand for PluginCommand {
    fn name(&self) -> &str {
        "plugin"
    }

    fn description(&self) -> &str {
        "Show plugin details"
    }

    fn usage(&self) -> &str {
        "plugin <id>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Plugins
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let id = match args.first() {
                Some(id) => *id,
                None => return CommandOutput::Error("Usage: plugin <id>".to_string()),
            };

            let pm = services.plugin_manager.read().await;
            let plugins = pm.list_plugins();

            let meta = match plugins.iter().find(|p| p.id == id) {
                Some(m) => m,
                None => return CommandOutput::Error(format!("Plugin '{id}' not found")),
            };

            let state = pm
                .plugin_state(id)
                .map(format_plugin_state)
                .unwrap_or_else(|| "Unknown".to_string());

            let authors = if meta.authors.is_empty() {
                "-".to_string()
            } else {
                meta.authors.join(", ")
            };

            let description = meta.description.as_deref().unwrap_or("-");

            let deps: Vec<String> = meta
                .dependencies
                .iter()
                .map(|d| {
                    if d.optional {
                        format!("{} (optional)", d.id)
                    } else {
                        d.id.clone()
                    }
                })
                .collect();
            let deps_str = if deps.is_empty() {
                "-".to_string()
            } else {
                deps.join(", ")
            };

            CommandOutput::Lines(vec![
                OutputLine::Info(format!("  Plugin: {}", meta.name)),
                OutputLine::Info(format!("  ID: {}", meta.id)),
                OutputLine::Info(format!("  Version: {}", meta.version)),
                OutputLine::Info(format!("  State: {state}")),
                OutputLine::Info(format!("  Authors: {authors}")),
                OutputLine::Info(format!("  Description: {description}")),
                OutputLine::Info(format!("  Dependencies: {deps_str}")),
            ])
        })
    }
}

fn format_plugin_state(state: &PluginState) -> String {
    match state {
        PluginState::Loading => "Loading".to_string(),
        PluginState::Enabled => "Enabled".to_string(),
        PluginState::Disabled => "Disabled".to_string(),
        PluginState::Error(e) => format!("Error: {e}"),
    }
}
