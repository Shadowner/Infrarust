use std::collections::HashMap;

use super::actor_pair::ActorPair;

/// Storage type alias for actors organized by config_id
pub type ActorStorage = HashMap<String, Vec<ActorPair>>;

/// Enum representing messages that can be sent to the supervisor
pub enum SupervisorMessage {
    Shutdown,
    Disconnect,
}

/// Statistics about tasks for a given configuration
#[derive(Debug, Clone)]
pub struct TaskStats {
    /// Configuration ID these tasks belong to
    pub config_id: String,
    /// Number of active actors for this configuration
    pub active_actor_count: usize,
    /// Total number of tasks registered
    pub task_count: usize,
    /// Number of tasks that are still running
    pub running_count: usize,
    /// Number of tasks that have completed
    pub completed_count: usize,
    /// Number of tasks that don't have associated actors (potential leak)
    pub orphaned_count: usize,
    /// Detailed information about individual tasks
    pub task_handles: Vec<TaskInfo>,
}

/// Information about an individual task
#[derive(Debug, Clone)]
pub struct TaskInfo {
    /// Task index in the handles array
    pub id: usize,
    /// Whether the task has finished execution
    pub is_finished: bool,
    /// Whether the task was aborted
    pub is_aborted: bool,
}

#[derive(Debug, Clone)]
pub struct ActorSupervisorMetrics {
    pub total_actor_entries: usize,
    pub actors_by_config: HashMap<String, usize>,
    pub total_task_entries: usize,
    pub tasks_by_config: HashMap<String, usize>,
    pub orphaned_tasks: usize,
}
