use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::{CommandContext, CommandSource};
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{PluginHealth, PluginRuntimeStatus};

use crate::commands::{ArgSpec, CommandServices, SubcommandAlias, SubcommandHandler};
use crate::services::command_manager::DispatchOutcome;
use crate::util::whole_seconds;

pub(crate) struct PluginSubcommand;

impl SubcommandHandler for PluginSubcommand {
    fn name(&self) -> &str {
        "plugin"
    }

    fn description(&self) -> &str {
        "Run a plugin command by namespace"
    }

    fn aliases(&self) -> &'static [SubcommandAlias] {
        &[SubcommandAlias {
            name: "plugins",
            description: "List loaded plugins",
            usage: "/ir plugins",
        }]
    }

    fn admin_only(&self) -> bool {
        true
    }

    fn usage(&self) -> &str {
        "/ir plugin <plugin_id> <command> [args...]"
    }

    fn runs_bare(&self) -> bool {
        false
    }

    fn arguments(&self) -> &'static [ArgSpec] {
        const {
            &[
                ArgSpec::word("plugin_id").needs_more(),
                ArgSpec::phrase("command").ask_server(),
            ]
        }
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let player = &ctx.source;

            match args.len() {
                0 => {
                    let plugins = services.plugin_registry.list_plugin_info();
                    if plugins.is_empty() {
                        player.send_message(ProxyMessage::info("No plugins loaded."));
                        return;
                    }
                    player
                        .send_message(ProxyMessage::info(&format!("Plugins ({}):", plugins.len())));
                    for info in &plugins {
                        let meta = &info.metadata;
                        let desc = meta.description.as_deref().unwrap_or("No description");
                        player.send_message(ProxyMessage::detail(&format!(
                            "  {} v{} - {}{}",
                            meta.name,
                            meta.version,
                            desc,
                            health_note(info.runtime.as_ref())
                        )));
                    }
                }
                1 => {
                    let plugin_id = &args[0];
                    let cmds = services.command_manager.commands_for_plugin(plugin_id);
                    if cmds.is_empty() {
                        player.send_message(ProxyMessage::error(&format!(
                            "Plugin '{}' not found or has no commands.",
                            plugin_id
                        )));
                        return;
                    }
                    player.send_message(ProxyMessage::info(&format!(
                        "Commands for plugin '{}':",
                        plugin_id
                    )));
                    for info in &cmds {
                        player.send_message(ProxyMessage::detail(&format!(
                            "  /{} - {}",
                            info.spec.name, info.spec.description
                        )));
                    }
                }
                _ => {
                    let plugin_id = &args[0];
                    let command_name = &args[1];
                    let input = namespaced_input(plugin_id, command_name, &args[2..]);
                    let outcome = services
                        .command_manager
                        .dispatch(player.clone(), &input)
                        .await;
                    if outcome == DispatchOutcome::Unknown {
                        player.send_message(ProxyMessage::error(&format!(
                            "Command '{}' not found for plugin '{}'.",
                            command_name, plugin_id
                        )));
                    }
                }
            }
        })
    }

    fn tab_complete<'a>(
        &'a self,
        args: &'a [String],
        source: &'a CommandSource,
        services: &'a CommandServices,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            match args.len() {
                0 | 1 => {
                    let prefix = args.first().map(String::as_str).unwrap_or("");
                    let plugins = services.plugin_registry.list_plugin_info();
                    plugins
                        .into_iter()
                        .map(|p| p.metadata.id)
                        .filter(|id| id.starts_with(prefix))
                        .collect()
                }
                2 => {
                    let plugin_id = args[0].as_str();
                    let prefix = args[1].as_str();
                    services
                        .command_manager
                        .commands_for_plugin(plugin_id)
                        .into_iter()
                        .map(|info| info.spec.name)
                        .filter(|name| name.starts_with(prefix))
                        .collect()
                }
                _ => {
                    let input = namespaced_input(&args[0], &args[1], &args[2..]);
                    services
                        .command_manager
                        .suggest(source.clone(), &input)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .map(|suggestion| suggestion.text)
                        .collect()
                }
            }
        })
    }
}

fn health_note(runtime: Option<&PluginRuntimeStatus>) -> String {
    let Some(runtime) = runtime else {
        return String::new();
    };
    match (runtime.health, runtime.health.retry_in()) {
        (PluginHealth::Healthy, _) => String::new(),
        (health, Some(retry_in)) => format!(
            " [{}, next attempt in {}s]",
            health.as_str(),
            whole_seconds(retry_in).as_secs()
        ),
        (health, None) => format!(" [{}]", health.as_str()),
    }
}

fn namespaced_input(plugin_id: &str, command: &str, rest: &[String]) -> String {
    format!("{plugin_id}:{command} {}", rest.join(" "))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use infrarust_api::plugin::PluginQueueStats;

    use super::*;

    fn status(health: PluginHealth) -> PluginRuntimeStatus {
        PluginRuntimeStatus::new(health, 2, PluginQueueStats::default())
    }

    #[test]
    fn only_an_unhealthy_supervised_plugin_gets_a_health_note() {
        assert_eq!(health_note(None), "");
        assert_eq!(health_note(Some(&status(PluginHealth::Healthy))), "");
        assert_eq!(
            health_note(Some(&status(PluginHealth::Recovering { retry_in: None }))),
            " [recovering]"
        );
        assert_eq!(
            health_note(Some(&status(PluginHealth::Quarantined {
                retry_in: Duration::from_millis(29_100)
            }))),
            " [quarantined, next attempt in 30s]"
        );
        assert_eq!(
            health_note(Some(&status(PluginHealth::Stopped))),
            " [stopped]"
        );
    }
}
