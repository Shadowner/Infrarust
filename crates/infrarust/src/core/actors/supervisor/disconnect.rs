use infrarust_config::LogType;
use tracing::{debug, info, instrument};

use super::{ActorSupervisor, actor_pair::ActorPair};

impl ActorSupervisor {
    pub(crate) async fn log_disconnect_if_needed(&self, pair: &ActorPair) {
        if !pair
            .disconnect_logged
            .load(std::sync::atomic::Ordering::SeqCst)
            && !pair.username.is_empty()
            && pair.created_at.elapsed().as_secs() > 5
        {
            info!(
                log_type = LogType::Supervisor.as_str(),
                "Player '{}' disconnected from server '{}' ({})",
                pair.username,
                pair.server_name,
                pair.config_id
            );

            let duration_secs = pair.created_at.elapsed().as_secs();
            debug!(
                log_type = LogType::Supervisor.as_str(),
                "Session duration for '{}': {} seconds", pair.username, duration_secs
            );
            pair.disconnect_logged
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    pub async fn log_player_disconnect(&self, session_id: uuid::Uuid, reason: &str) {
        let mut actors_to_remove = Vec::new();
        let mut config_ids_to_clean = Vec::new();

        {
            let mut actors = self.actors.write().await;
            for (config_id, pairs) in actors.iter_mut() {
                let mut disconnect_indexes = Vec::new();

                for (idx, pair) in pairs.iter().enumerate() {
                    if pair.session_id == session_id {
                        if pair.is_login && !pair.username.is_empty() {
                            if !pair
                                .disconnect_logged
                                .load(std::sync::atomic::Ordering::SeqCst)
                            {
                                info!(
                                    log_type = LogType::Supervisor.as_str(),
                                    "Player '{}' disconnected from server '{}' ({}) - reason: {}",
                                    pair.username,
                                    pair.server_name,
                                    config_id,
                                    reason
                                );

                                let duration_secs = pair.created_at.elapsed().as_secs();
                                debug!(
                                    log_type = LogType::Supervisor.as_str(),
                                    "Session duration for '{}': {} seconds",
                                    pair.username,
                                    duration_secs
                                );

                                pair.disconnect_logged
                                    .store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                        } else {
                            debug!(
                                log_type = LogType::Supervisor.as_str(),
                                "Status Request connection disconnected from server '{}' ({}) - reason: {}",
                                pair.server_name,
                                config_id,
                                reason
                            );
                        }

                        pair.shutdown
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        disconnect_indexes.push(idx);
                    }
                }

                if !disconnect_indexes.is_empty() {
                    config_ids_to_clean.push(config_id.clone());
                }

                disconnect_indexes.sort_unstable_by(|a, b| b.cmp(a));
                for idx in disconnect_indexes {
                    if idx < pairs.len() {
                        // Track session_id and config_id for cleanup
                        if let Some(removed_pair) = pairs.get(idx) {
                            // Only track login sessions for telemetry updates
                            if removed_pair.is_login {
                                actors_to_remove.push((removed_pair.session_id, config_id.clone()));
                            }
                        }
                        pairs.remove(idx);
                    }
                }
            }
        }

        if !config_ids_to_clean.is_empty() {
            let actor_counts: std::collections::HashMap<String, usize> = {
                let actors = self.actors.read().await;
                config_ids_to_clean
                    .iter()
                    .map(|config_id| {
                        let count = actors.get(config_id).map_or(0, |pairs| pairs.len());
                        (config_id.clone(), count)
                    })
                    .collect()
            };

            let mut tasks = self.tasks.write().await;

            for config_id in config_ids_to_clean {
                if let Some(task_handles) = tasks.get_mut(&config_id) {
                    let actors_count = actor_counts.get(&config_id).copied().unwrap_or(0);

                    while task_handles.len() > actors_count {
                        if let Some(handle) = task_handles.pop() {
                            debug!(
                                log_type = LogType::Supervisor.as_str(),
                                "Aborting task for disconnected session in {}", config_id
                            );
                            handle.abort();
                        }
                    }
                }
            }
        }

        #[cfg(feature = "telemetry")]
        for (session_id, config_id) in actors_to_remove {
            crate::telemetry::TELEMETRY.update_player_count(-1, &config_id, session_id, "");
        }

        self.check_and_mark_empty_servers().await;
        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Cleanup completed for session {}", session_id
        );
    }
}
