use std::sync::Arc;

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::shared_component::SharedComponent;
use tracing::debug;

pub struct TasksCommand {
    shared: Arc<SharedComponent>,
}

impl TasksCommand {
    pub fn new(shared: Arc<SharedComponent>) -> Self {
        Self { shared }
    }

    async fn format_task_info(&self) -> String {
        let mut result = String::new();
        let supervisor = self.shared.actor_supervisor();

        // Get task statistics
        let task_stats = supervisor.get_task_statistics().await;

        if task_stats.is_empty() {
            return fmt::warning("No tasks currently registered.").to_string();
        }

        result.push_str(&format!("{}\n\n", fmt::header("Task Monitor")));

        // Summary section
        let total_tasks: usize = task_stats.values().map(|s| s.task_count).sum();
        let running_tasks: usize = task_stats.values().map(|s| s.running_count).sum();
        let completed_tasks: usize = task_stats.values().map(|s| s.completed_count).sum();
        let orphaned_tasks: usize = task_stats.values().map(|s| s.orphaned_count).sum();

        result.push_str(&format!(
            "{}: {} total, {} running, {} completed, {} orphaned\n\n",
            fmt::sub_header("Summary"),
            total_tasks,
            fmt::entity(&running_tasks.to_string()),
            completed_tasks,
            if orphaned_tasks > 0 {
                fmt::error(&orphaned_tasks.to_string())
            } else {
                orphaned_tasks.to_string()
            }
        ));

        // Per-config details
        for stats in task_stats.values() {
            let health_indicator = if stats.orphaned_count > 0 {
                fmt::error("WARNING: ORPHANED TASKS")
            } else if stats.active_actor_count == 0 && stats.task_count > 0 {
                fmt::warning("No active actors")
            } else if stats.running_count > stats.active_actor_count * 2 {
                fmt::warning("Many tasks per actor")
            } else {
                fmt::success("Healthy")
            };

            result.push_str(&format!(
                "{} {} - {} tasks, {} actors - {}\n",
                fmt::sub_header("Config"),
                fmt::entity(&stats.config_id),
                stats.task_count,
                stats.active_actor_count,
                health_indicator
            ));

            if stats.task_count > 0 {
                result.push_str("  Tasks:\n");
                result.push_str(&format!(
                    "    {} running, {} completed\n",
                    stats.running_count, stats.completed_count
                ));

                // For configs with potential issues, show detailed task list
                if stats.orphaned_count > 0 || stats.running_count > stats.active_actor_count * 2 {
                    result.push_str("  Task details:\n");
                    for task in &stats.task_handles {
                        let status = if task.is_finished {
                            if task.is_aborted {
                                fmt::warning("ABORTED")
                            } else {
                                fmt::secondary("completed")
                            }
                        } else {
                            fmt::entity("running")
                        };

                        result.push_str(&format!("    Task #{}: {}\n", task.id, status));
                    }
                }
            }

            result.push('\n');
        }

        // Add memory cleanup recommendation if needed
        if orphaned_tasks > 0 {
            result.push_str(&format!(
                "{} {} orphaned tasks detected. Run 'cleanup' to remove them.\n",
                fmt::error("WARNING:"),
                orphaned_tasks
            ));
        }

        result
    }
}

impl Command for TasksCommand {
    fn name(&self) -> &'static str {
        "tasks"
    }

    fn description(&self) -> &'static str {
        "Shows detailed information about background tasks and their status"
    }

    fn execute(&self, _args: Vec<String>) -> CommandFuture {
        debug!("Executing tasks command");
        let shared = self.shared.clone();

        Box::pin(async move {
            let tasks_cmd = TasksCommand { shared };
            tasks_cmd.format_task_info().await
        })
    }
}
