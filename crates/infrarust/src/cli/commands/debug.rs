use std::sync::Arc;
use std::time::Duration;

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::shared_component::SharedComponent;
use infrarust_config::LogType;
use tracing::debug;

const NA: &str = "N/A";

pub struct DebugCommand {
    shared: Arc<SharedComponent>,
}

impl DebugCommand {
    pub fn new(shared: Arc<SharedComponent>) -> Self {
        Self { shared }
    }

    async fn format_debug_info(&self) -> String {
        let mut result = String::new();
        let supervisor = self.shared.actor_supervisor();

        // Get raw access to the internal state for debugging
        let actors_data = supervisor.get_all_actors().await;

        result.push_str(&format!(
            "{}\n\n",
            fmt::header("Actor and Task Debug Information")
        ));

        // Count actors by config
        let mut total_actors = 0;
        for (config_id, actors) in &actors_data {
            let actor_count = actors.len();
            total_actors += actor_count;

            result.push_str(&format!(
                "{} {} - {} actors\n",
                fmt::sub_header("Config"),
                fmt::entity(config_id),
                actor_count
            ));

            // List actors with detailed info
            for (i, actor) in actors.iter().enumerate() {
                let alive_duration = format_duration(actor.created_at.elapsed());
                let is_shutdown = actor.shutdown.load(std::sync::atomic::Ordering::SeqCst);

                result.push_str(&format!(
                    "  {}. {} - Session: {} - Age: {} - {}\n",
                    i + 1,
                    if actor.username.is_empty() {
                        fmt::secondary("<status>")
                    } else {
                        fmt::entity(&actor.username)
                    },
                    fmt::id(&actor.session_id.to_string()),
                    fmt::secondary(&alive_duration),
                    if is_shutdown {
                        fmt::warning("SHUTDOWN")
                    } else {
                        fmt::success("ACTIVE")
                    }
                ));
            }
            result.push('\n');
        }

        result.push_str(&self.format_memory_metrics());

        // Memory usage information (if available on the platform)
        if let Some(usage) = get_memory_usage() {
            result.push_str(&format!(
                "\n{}\n",
                fmt::sub_header(&format!("Current process memory usage: {:.2} MB", usage))
            ));
        }

        result.push_str(&format!(
            "\n{}\n",
            fmt::header(&format!("Total Actors: {}", total_actors))
        ));

        result
    }

    fn format_memory_metrics(&self) -> String {
        let mut result = String::new();

        result.push_str(&format!("{}\n\n", fmt::header("Memory Retention Metrics")));

        result.push_str(&format!("{}\n", fmt::warning("[CRITICAL]")));

        // ActorSupervisor metrics
        let supervisor = self.shared.actor_supervisor();
        if let Some(metrics) = supervisor.get_memory_metrics() {
            result.push_str(&format!(
                "  ActorSupervisor :: actors = {}\n",
                metrics.total_actor_entries
            ));
            for (config_id, count) in &metrics.actors_by_config {
                if *count > 0 {
                    result.push_str(&format!("    - {}: {}\n", fmt::entity(config_id), count));
                }
            }

            result.push_str(&format!(
                "  ActorSupervisor :: tasks = {}\n",
                metrics.total_task_entries
            ));
            for (config_id, count) in &metrics.tasks_by_config {
                if *count > 0 {
                    result.push_str(&format!("    - {}: {}\n", fmt::entity(config_id), count));
                }
            }

            result.push_str(&format!(
                "  ActorSupervisor :: orphaned_tasks = {}\n",
                if metrics.orphaned_tasks > 0 {
                    fmt::warning(&metrics.orphaned_tasks.to_string())
                } else {
                    fmt::success(&metrics.orphaned_tasks.to_string())
                }
            ));
        } else {
            result.push_str(&format!("  ActorSupervisor :: actors = {}\n", NA));
            result.push_str(&format!("  ActorSupervisor :: tasks = {}\n", NA));
            result.push_str(&format!("  ActorSupervisor :: orphaned_tasks = {}\n", NA));
        }

        // RateLimiter metrics
        if let Some(rate_limiter_metrics) = self.shared.filter_registry().get_rate_limiter_metrics()
        {
            for (name, counter_size) in rate_limiter_metrics {
                match counter_size {
                    Some(size) => {
                        result.push_str(&format!(
                            "  RateLimiter ({}) :: counters = {} {}\n",
                            fmt::entity(&name),
                            size,
                            fmt::secondary("(no limit)")
                        ));
                    }
                    None => {
                        result.push_str(&format!(
                            "  RateLimiter ({}) :: counters = {}\n",
                            fmt::entity(&name),
                            NA
                        ));
                    }
                }
            }
        } else {
            result.push_str(&format!("  RateLimiter :: counters = {}\n", NA));
        }

        result.push('\n');

        result.push_str(&format!("{}\n", fmt::warning("[HIGH]")));

        // Gateway metrics
        if let Some(gateway) = self.shared.gateway() {
            if let Some(metrics) = gateway.get_memory_metrics() {
                result.push_str(&format!(
                    "  Gateway :: pending_status_requests = {}\n",
                    metrics.pending_status_requests_count
                ));
            } else {
                result.push_str(&format!("  Gateway :: pending_status_requests = {}\n", NA));
            }
        } else {
            result.push_str(&format!("  Gateway :: pending_status_requests = {}\n", NA));
        }

        // Manager metrics
        if let Some(metrics) = self.shared.server_managers().get_memory_metrics() {
            result.push_str(&format!(
                "  Manager :: starting_servers = {}\n",
                metrics.starting_servers_count
            ));
        } else {
            result.push_str(&format!("  Manager :: starting_servers = {}\n", NA));
        }

        result.push('\n');

        result.push_str(&format!("{}\n", fmt::secondary("[MEDIUM]")));

        // Gateway status cache
        if let Some(gateway) = self.shared.gateway() {
            if let Some(metrics) = gateway.get_memory_metrics() {
                result.push_str(&format!(
                    "  Gateway :: status_cache_entries = {} / {}\n",
                    metrics.status_cache_entries, metrics.status_cache_max_size
                ));
            } else {
                result.push_str(&format!("  Gateway :: status_cache_entries = {}\n", NA));
            }
        } else {
            result.push_str(&format!("  Gateway :: status_cache_entries = {}\n", NA));
        }

        // Manager tracking maps
        if let Some(metrics) = self.shared.server_managers().get_memory_metrics() {
            result.push_str(&format!(
                "  Manager :: time_since_empty = {}\n",
                metrics.time_since_empty_count
            ));
            result.push_str(&format!(
                "  Manager :: shutdown_tasks = {}\n",
                metrics.shutdown_tasks_count
            ));
            result.push_str(&format!(
                "  Manager :: shutdown_timers = {}\n",
                metrics.shutdown_timers_count
            ));
        } else {
            result.push_str(&format!("  Manager :: time_since_empty = {}\n", NA));
            result.push_str(&format!("  Manager :: shutdown_tasks = {}\n", NA));
            result.push_str(&format!("  Manager :: shutdown_timers = {}\n", NA));
        }

        // ConfigurationService metrics
        if let Some(count) = self.shared.configuration_service().config_count() {
            result.push_str(&format!(
                "  ConfigurationService :: configurations = {}\n",
                count
            ));
        } else {
            result.push_str(&format!(
                "  ConfigurationService :: configurations = {}\n",
                NA
            ));
        }

        // FilterRegistry metrics
        if let Some(count) = self.shared.filter_registry().filter_count() {
            result.push_str(&format!(
                "  FilterRegistry :: registered_filters = {}\n",
                count
            ));
        } else {
            result.push_str(&format!(
                "  FilterRegistry :: registered_filters = {}\n",
                NA
            ));
        }

        result
    }
}

impl Command for DebugCommand {
    fn name(&self) -> &'static str {
        "debug"
    }

    fn description(&self) -> &'static str {
        "Shows detailed debug information about active actors and tasks"
    }
    fn execute(&self, _args: Vec<String>) -> CommandFuture {
        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Executing debug command"
        );
        let shared = self.shared.clone();

        Box::pin(async move {
            let debug_cmd = DebugCommand { shared };
            debug_cmd.format_debug_info().await
        })
    }
}

// Helper function to format duration in a human-readable way
fn format_duration(duration: Duration) -> String {
    if duration.as_secs() < 60 {
        format!("{}s", duration.as_secs())
    } else if duration.as_secs() < 3600 {
        format!("{}m {}s", duration.as_secs() / 60, duration.as_secs() % 60)
    } else {
        format!(
            "{}h {}m {}s",
            duration.as_secs() / 3600,
            (duration.as_secs() % 3600) / 60,
            duration.as_secs() % 60
        )
    }
}

// Helper function to get current process memory usage
#[cfg(target_os = "linux")]
fn get_memory_usage() -> Option<f64> {
    use std::fs::File;
    use std::io::Read;

    let mut status = String::new();
    if File::open("/proc/self/status")
        .ok()?
        .read_to_string(&mut status)
        .is_ok()
    {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2
                    && let Ok(kb) = parts[1].parse::<f64>()
                {
                    return Some(kb / 1024.0); // Convert KB to MB
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn get_memory_usage() -> Option<f64> {
    // On Windows or other platforms, we need to use platform-specific APIs
    // This is a simplified version that doesn't report memory usage
    None
}
