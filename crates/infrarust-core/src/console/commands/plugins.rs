use std::future::Future;
use std::pin::Pin;

use infrarust_api::plugin::PluginMetadata;

use crate::console::ConsoleServices;
use crate::console::commands::usage;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{Block, CommandCategory, CommandOutput, Fields, Span, Table};
use crate::plugin::PluginState;
use crate::terminal::Mark;

pub(crate) fn plugin_state_span(state: Option<&PluginState>) -> Span {
    match state {
        Some(PluginState::Enabled) => Span::marked(Mark::Up, "enabled"),
        Some(PluginState::Disabled) => Span::marked(Mark::Idle, "disabled"),
        Some(PluginState::Loading) => Span::marked(Mark::Busy, "loading"),
        Some(PluginState::Error(_)) => Span::marked(Mark::Down, "error"),
        Some(other) => Span::muted(other.as_str()),
        None => Span::muted("unknown"),
    }
}

fn listed(items: impl IntoIterator<Item = String>) -> String {
    let joined = items.into_iter().collect::<Vec<_>>().join(", ");
    if joined.is_empty() {
        "-".to_string()
    } else {
        joined
    }
}

pub(crate) fn plugins_output(plugins: &[(&PluginMetadata, Option<&PluginState>)]) -> CommandOutput {
    if plugins.is_empty() {
        return CommandOutput::Note("No plugins loaded".to_string());
    }

    let mut table = Table::new(&["ID", "Name", "Version", "State"]);
    for (meta, state) in plugins {
        table.row([
            Span::entity(meta.id.as_str()),
            Span::plain(meta.name.as_str()),
            Span::muted(meta.version.as_str()),
            plugin_state_span(*state),
        ]);
    }

    Block::new("Plugins")
        .meta(format!("{} loaded", plugins.len()))
        .table(table)
        .into()
}

pub(crate) fn plugin_block(meta: &PluginMetadata, state: Option<&PluginState>) -> Block {
    let dependencies = meta.dependencies.iter().map(|dependency| {
        if dependency.optional {
            format!("{} (optional)", dependency.id)
        } else {
            dependency.id.clone()
        }
    });

    let mut fields = Fields::new()
        .field("id", Span::entity(meta.id.as_str()))
        .field("version", meta.version.as_str())
        .field("authors", listed(meta.authors.iter().cloned()))
        .field("description", meta.description.as_deref().unwrap_or("-"))
        .field("dependencies", listed(dependencies));
    if let Some(PluginState::Error(error)) = state {
        fields.push("error", Span::err(error.as_str()));
    }

    Block::new(meta.name.as_str())
        .meta(plugin_state_span(state))
        .fields(fields)
}

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
            let manager = services.plugin_manager.read().await;
            let plugins: Vec<_> = manager
                .list_plugins()
                .into_iter()
                .map(|meta| (meta, manager.plugin_state(&meta.id)))
                .collect();
            plugins_output(&plugins)
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
            let Some(id) = args.first() else {
                return usage(self);
            };

            let manager = services.plugin_manager.read().await;
            let Some(meta) = manager.list_plugins().into_iter().find(|p| p.id == *id) else {
                return CommandOutput::error(format!("Plugin '{id}' not found"));
            };

            plugin_block(meta, manager.plugin_state(id)).into()
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use infrarust_api::plugin::PluginDependency;

    use super::*;
    use crate::console::render::Renderer;

    fn render(output: &CommandOutput) -> String {
        Renderer::new(false, None).render(output)
    }

    fn metadata(id: &str, name: &str, version: &str) -> PluginMetadata {
        PluginMetadata::new(id, name, version)
    }

    #[test]
    fn states_map_to_marked_spans() {
        assert_eq!(
            plugin_state_span(Some(&PluginState::Enabled)),
            Span::marked(Mark::Up, "enabled")
        );
        assert_eq!(
            plugin_state_span(Some(&PluginState::Disabled)),
            Span::marked(Mark::Idle, "disabled")
        );
        assert_eq!(
            plugin_state_span(Some(&PluginState::Loading)),
            Span::marked(Mark::Busy, "loading")
        );
        assert_eq!(
            plugin_state_span(Some(&PluginState::Error("boom".into()))),
            Span::marked(Mark::Down, "error")
        );
        assert_eq!(plugin_state_span(None), Span::muted("unknown"));
    }

    #[test]
    fn plugins_are_listed_with_their_state() {
        let auth = metadata("auth", "Auth", "1.2.0");
        let motd = metadata("motd", "Dynamic MOTD", "0.3.1");
        let enabled = PluginState::Enabled;
        let failed = PluginState::Error("missing config".into());
        assert_eq!(
            render(&plugins_output(&[
                (&auth, Some(&enabled)),
                (&motd, Some(&failed))
            ])),
            "# Plugins - 2 loaded\n\
             | ID     NAME           VERSION   STATE\n\
             | auth   Auth           1.2.0     enabled\n\
             | motd   Dynamic MOTD   0.3.1     error"
        );
    }

    #[test]
    fn no_plugins_is_a_note() {
        assert_eq!(render(&plugins_output(&[])), "- No plugins loaded");
    }

    #[test]
    fn a_plugin_shows_its_details_and_error() {
        let mut meta = metadata("motd", "Dynamic MOTD", "0.3.1");
        meta.authors = vec!["alice".into(), "bob".into()];
        meta.description = Some("Rotates the server list message".into());
        meta.dependencies = vec![
            PluginDependency {
                id: "auth".into(),
                optional: false,
            },
            PluginDependency {
                id: "stats".into(),
                optional: true,
            },
        ];
        let state = PluginState::Error("missing config".into());
        assert_eq!(
            render(&plugin_block(&meta, Some(&state)).into()),
            "# Dynamic MOTD - error\n\
             | id            motd\n\
             | version       0.3.1\n\
             | authors       alice, bob\n\
             | description   Rotates the server list message\n\
             | dependencies  auth, stats (optional)\n\
             | error         missing config"
        );
    }

    #[test]
    fn a_bare_plugin_fills_missing_details_with_dashes() {
        let meta = metadata("auth", "Auth", "1.2.0");
        assert_eq!(
            render(&plugin_block(&meta, Some(&PluginState::Enabled)).into()),
            "# Auth - enabled\n\
             | id            auth\n\
             | version       1.2.0\n\
             | authors       -\n\
             | description   -\n\
             | dependencies  -"
        );
    }
}
