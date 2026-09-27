use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use infrarust_api::plugin::{PluginHealth, PluginMetadata, PluginRuntimeStatus};

use crate::console::ConsoleServices;
use crate::console::commands::usage;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{Block, CommandCategory, CommandOutput, Fields, Line, Span, Table};
use crate::console::parser::format_duration_short;
use crate::plugin::PluginState;
use crate::terminal::Mark;

pub(crate) type PluginRow<'a> = (
    &'a PluginMetadata,
    Option<&'a PluginState>,
    Option<PluginRuntimeStatus>,
);

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

fn health_mark(health: &PluginHealth) -> Mark {
    match health {
        PluginHealth::Healthy => Mark::Up,
        PluginHealth::Recovering { .. } => Mark::Busy,
        PluginHealth::Quarantined { .. } => Mark::Down,
        _ => Mark::Idle,
    }
}

fn whole_seconds(duration: Duration) -> Duration {
    let rounded_up = u64::from(duration.subsec_nanos() > 0);
    Duration::from_secs(duration.as_secs().saturating_add(rounded_up))
}

fn health_span(runtime: Option<&PluginRuntimeStatus>) -> Span {
    let Some(runtime) = runtime else {
        return Span::muted("-");
    };
    let health = &runtime.health;
    let label = match health.retry_in() {
        Some(retry_in) => format!(
            "{} {}",
            health.as_str(),
            format_duration_short(whole_seconds(retry_in))
        ),
        None => health.as_str().to_owned(),
    };
    Span::marked(health_mark(health), label)
}

fn health_line(health: &PluginHealth) -> Line {
    let line = Line::new().push(Span::marked(health_mark(health), health.as_str()));
    match health.retry_in() {
        Some(retry_in) => line.push(format!(
            ", next attempt in {}",
            format_duration_short(whole_seconds(retry_in))
        )),
        None => line,
    }
}

fn short_duration(duration: Duration) -> String {
    let micros = duration.as_micros();
    if micros < 1_000 {
        format!("{micros}µs")
    } else if micros < 1_000_000 {
        format!("{:.1}ms", duration.as_secs_f64() * 1_000.0)
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    }
}

fn wait_p99_span(runtime: Option<&PluginRuntimeStatus>) -> Span {
    match runtime {
        Some(runtime) if runtime.queue.recent.taken > 0 => {
            Span::plain(short_duration(runtime.queue.recent.wait_p99))
        }
        _ => Span::muted("-"),
    }
}

fn runtime_fields(fields: &mut Fields, runtime: &PluginRuntimeStatus) {
    let queue = &runtime.queue;
    let recent = &queue.recent;
    let span = format_duration_short(recent.span);
    fields.push("health", health_line(&runtime.health));
    fields.push("generation", runtime.generation.to_string());
    fields.push(
        "restarts",
        format!(
            "{} of {} in the last {}",
            runtime.restarts.in_window,
            runtime.restarts.max,
            format_duration_short(runtime.restarts.window)
        ),
    );
    let last_fault = match &runtime.last_fault {
        Some(fault) => Line::new()
            .push(Span::err(fault.cause.as_str()))
            .push(Span::muted(format!(
                " ({} ago, generation {})",
                format_duration_short(fault.ago),
                fault.generation
            ))),
        None => Span::muted("none").into(),
    };
    fields.push("last fault", last_fault);
    fields.push(
        "queue",
        format!(
            "{} of {} waiting, at most {} in the last {span}",
            queue.depth, queue.capacity, recent.peak_depth
        ),
    );
    let waits = if recent.taken == 0 {
        Span::muted(format!("no calls in the last {span}"))
    } else {
        Span::plain(format!(
            "p50 {}, p99 {}, max {} over {} calls in the last {span}",
            short_duration(recent.wait_p50),
            short_duration(recent.wait_p99),
            short_duration(recent.wait_max),
            recent.taken
        ))
    };
    fields.push("queue wait", waits);
}

fn listed(items: impl IntoIterator<Item = String>) -> String {
    let joined = items.into_iter().collect::<Vec<_>>().join(", ");
    if joined.is_empty() {
        "-".to_string()
    } else {
        joined
    }
}

pub(crate) fn plugins_output(plugins: &[PluginRow<'_>]) -> CommandOutput {
    if plugins.is_empty() {
        return CommandOutput::Note("No plugins loaded".to_string());
    }

    let supervised = plugins.iter().any(|(_, _, runtime)| runtime.is_some());
    let mut headers = vec!["ID", "Name", "Version", "State"];
    if supervised {
        headers.extend(["Health", "Wait p99"]);
    }
    let mut table = Table::new(&headers);
    for (meta, state, runtime) in plugins {
        let mut cells = vec![
            Span::entity(meta.id.as_str()),
            Span::plain(meta.name.as_str()),
            Span::muted(meta.version.as_str()),
            plugin_state_span(*state),
        ];
        if supervised {
            cells.push(health_span(runtime.as_ref()));
            cells.push(wait_p99_span(runtime.as_ref()));
        }
        table.row(cells);
    }

    Block::new("Plugins")
        .meta(format!("{} loaded", plugins.len()))
        .table(table)
        .into()
}

pub(crate) fn plugin_block(
    meta: &PluginMetadata,
    state: Option<&PluginState>,
    runtime: Option<&PluginRuntimeStatus>,
) -> Block {
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
    if let Some(runtime) = runtime {
        runtime_fields(&mut fields, runtime);
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
                .map(|meta| {
                    (
                        meta,
                        manager.plugin_state(&meta.id),
                        manager.plugin_runtime(&meta.id),
                    )
                })
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

            let runtime = manager.plugin_runtime(id);
            plugin_block(meta, manager.plugin_state(id), runtime.as_ref()).into()
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use infrarust_api::plugin::{
        PluginDependency, PluginFault, PluginQueueStats, PluginRestarts, QueueWindow,
    };

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
                (&auth, Some(&enabled), None),
                (&motd, Some(&failed), None)
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
            render(&plugin_block(&meta, Some(&state), None).into()),
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
            render(&plugin_block(&meta, Some(&PluginState::Enabled), None).into()),
            "# Auth - enabled\n\
             | id            auth\n\
             | version       1.2.0\n\
             | authors       -\n\
             | description   -\n\
             | dependencies  -"
        );
    }

    fn runtime(health: PluginHealth, generation: u64, recent: QueueWindow) -> PluginRuntimeStatus {
        PluginRuntimeStatus::new(health, generation, PluginQueueStats::new(3, 1024, recent))
            .with_restarts(PluginRestarts::new(2, 2, Duration::from_secs(300)))
    }

    fn busy_minute() -> QueueWindow {
        QueueWindow::new(Duration::from_secs(60), 1234, 12).waits(
            Duration::from_micros(21),
            Duration::from_micros(1_250),
            Duration::from_millis(3_400),
        )
    }

    #[test]
    fn health_maps_to_marked_spans_with_the_time_to_the_next_attempt() {
        let window = QueueWindow::default();
        let span = |health| health_span(Some(&runtime(health, 1, window)));
        assert_eq!(
            span(PluginHealth::Healthy),
            Span::marked(Mark::Up, "healthy")
        );
        assert_eq!(
            span(PluginHealth::Recovering { retry_in: None }),
            Span::marked(Mark::Busy, "recovering")
        );
        assert_eq!(
            span(PluginHealth::Recovering {
                retry_in: Some(Duration::from_millis(400))
            }),
            Span::marked(Mark::Busy, "recovering 1s")
        );
        assert_eq!(
            span(PluginHealth::Quarantined {
                retry_in: Duration::from_millis(11_200)
            }),
            Span::marked(Mark::Down, "quarantined 12s")
        );
        assert_eq!(
            span(PluginHealth::Stopped),
            Span::marked(Mark::Idle, "stopped")
        );
        assert_eq!(health_span(None), Span::muted("-"));
    }

    #[test]
    fn a_supervised_plugin_adds_health_and_wait_columns_and_a_native_one_fills_them_with_dashes() {
        let auth = metadata("auth", "Auth", "1.2.0");
        let flaky = metadata("flaky", "Flaky", "0.1.0");
        let enabled = PluginState::Enabled;
        let quarantined = runtime(
            PluginHealth::Quarantined {
                retry_in: Duration::from_secs(12),
            },
            7,
            busy_minute(),
        );
        assert_eq!(
            render(&plugins_output(&[
                (&auth, Some(&enabled), None),
                (&flaky, Some(&enabled), Some(quarantined)),
            ])),
            "# Plugins - 2 loaded\n\
             | ID      NAME    VERSION   STATE     HEALTH            WAIT P99\n\
             | auth    Auth    1.2.0     enabled   -                 -\n\
             | flaky   Flaky   0.1.0     enabled   quarantined 12s   1.2ms"
        );
    }

    #[test]
    fn a_supervised_plugin_block_shows_health_restarts_last_fault_and_queue() {
        let meta = metadata("flaky", "Flaky", "0.1.0");
        let quarantined = runtime(
            PluginHealth::Quarantined {
                retry_in: Duration::from_secs(12),
            },
            7,
            busy_minute(),
        )
        .with_last_fault(PluginFault::new(
            "the guest trapped: panicked at src/lib.rs:12:5: boom",
            Duration::from_secs(4),
            7,
        ));
        assert_eq!(
            render(&plugin_block(&meta, Some(&PluginState::Enabled), Some(&quarantined)).into()),
            "# Flaky - enabled\n\
             | id            flaky\n\
             | version       0.1.0\n\
             | authors       -\n\
             | description   -\n\
             | dependencies  -\n\
             | health        quarantined, next attempt in 12s\n\
             | generation    7\n\
             | restarts      2 of 2 in the last 5m\n\
             | last fault    the guest trapped: panicked at src/lib.rs:12:5: boom (4s ago, generation 7)\n\
             | queue         3 of 1024 waiting, at most 12 in the last 1m\n\
             | queue wait    p50 21µs, p99 1.2ms, max 3.4s over 1234 calls in the last 1m"
        );
    }

    #[test]
    fn an_idle_supervised_plugin_says_it_took_no_calls() {
        let meta = metadata("quiet", "Quiet", "0.1.0");
        let idle = runtime(
            PluginHealth::Healthy,
            1,
            QueueWindow::new(Duration::from_secs(60), 0, 0),
        );
        let rendered =
            render(&plugin_block(&meta, Some(&PluginState::Enabled), Some(&idle)).into());
        assert!(
            rendered.ends_with(
                "| health        healthy\n\
                 | generation    1\n\
                 | restarts      2 of 2 in the last 5m\n\
                 | last fault    none\n\
                 | queue         3 of 1024 waiting, at most 0 in the last 1m\n\
                 | queue wait    no calls in the last 1m"
            ),
            "{rendered}"
        );
    }
}
