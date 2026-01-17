use infrarust_config::LogType;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use super::ActorSupervisor;

impl ActorSupervisor {
    pub async fn shutdown_actors(&self, config_id: &str) {
        let mut actors = self.actors.write().await;
        if let Some(pairs) = actors.get_mut(config_id) {
            for pair in pairs.iter() {
                debug!(
                    log_type = LogType::Supervisor.as_str(),
                    "Shutting down actor for user {}", pair.username
                );
                pair.shutdown
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            pairs.clear();
        }

        let mut tasks = self.tasks.write().await;
        if let Some(task_handles) = tasks.remove(config_id) {
            for handle in task_handles {
                handle.abort();
            }
        }
    }

    pub async fn register_task(&self, config_id: &str, handle: JoinHandle<()>) {
        let mut tasks = self.tasks.write().await;
        tasks
            .entry(config_id.to_string())
            .or_insert_with(Vec::new)
            .push(handle);
    }

    pub async fn health_check(&self) {
        let mut actors = self.actors.write().await;
        let mut tasks = self.tasks.write().await;

        for (config_id, pairs) in actors.iter_mut() {
            let before_count = pairs.len();

            // Log any player disconnections before removing them
            for pair in pairs.iter() {
                if pair.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                    self.log_disconnect_if_needed(pair).await;
                }
            }

            // Remove actors with shutdown flag set
            pairs.retain(|pair| !pair.shutdown.load(std::sync::atomic::Ordering::SeqCst));

            let after_count = pairs.len();
            if before_count != after_count {
                debug!(
                    log_type = LogType::Supervisor.as_str(),
                    "Cleaned up {} dead actors for config {}",
                    before_count - after_count,
                    config_id
                );

                // Clean up any associated tasks
                if let Some(task_handles) = tasks.get_mut(config_id) {
                    while task_handles.len() > pairs.len() {
                        if let Some(handle) = task_handles.pop() {
                            debug!(
                                log_type = LogType::Supervisor.as_str(),
                                "Aborting orphaned task for {}", config_id
                            );
                            handle.abort();
                        }
                    }
                }
            }
        }

        // Check for stale tasks without associated actors
        tasks.retain(|config_id, handles| {
            if !actors.contains_key(config_id) || actors[config_id].is_empty() {
                for handle in handles.iter() {
                    debug!(
                        log_type = LogType::Supervisor.as_str(),
                        "Aborting orphaned task for {}", config_id
                    );
                    handle.abort();
                }
                false
            } else {
                true
            }
        });
    }

    /// Shutdown all actors across all servers
    pub async fn shutdown_all_actors(&self) {
        info!(
            log_type = LogType::Supervisor.as_str(),
            "Shutting down all actors"
        );
        let mut actors = self.actors.write().await;

        for (config_id, pairs) in actors.iter_mut() {
            for pair in pairs.iter() {
                debug!(
                    log_type = LogType::Supervisor.as_str(),
                    "Shutting down actor for user {} on {}", pair.username, config_id
                );
                pair.shutdown
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        // Give actors time to clean up
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        // Clear all actors
        actors.clear();

        // Also clean up tasks
        let mut tasks = self.tasks.write().await;
        for (config_id, handles) in tasks.iter_mut() {
            debug!(
                log_type = LogType::Supervisor.as_str(),
                "Aborting {} tasks for {}",
                handles.len(),
                config_id
            );
            for handle in handles.iter() {
                handle.abort();
            }
        }
        tasks.clear();

        info!(
            log_type = LogType::Supervisor.as_str(),
            "All actors have been shut down"
        );
    }
}
