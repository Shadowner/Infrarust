use std::sync::Arc;
use std::time::Duration;

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::shared_component::SharedComponent;
use tracing::debug;

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

        // Memory usage information (if available on the platform)
        if let Some(usage) = get_memory_usage() {
            result.push_str(&format!(
                "{}\n",
                fmt::sub_header(&format!("Current process memory usage: {:.2} MB", usage))
            ));
        }

        result.push_str(&format!(
            "{}\n",
            fmt::header(&format!("Total Actors: {}", total_actors))
        ));

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
        debug!("Executing debug command");
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
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<f64>() {
                        return Some(kb / 1024.0); // Convert KB to MB
                    }
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
