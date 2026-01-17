use std::{collections::HashMap, time::Duration};

use infrarust_config::{LogType, ServerManagerConfig};
use tracing::debug;

use super::ActorSupervisor;

impl ActorSupervisor {
    pub async fn check_and_mark_empty_servers(&self) {
        if let Some(server_manager) = &self.server_manager {
            debug!(
                log_type = LogType::ServerManager.as_str(),
                "Checking for empty servers"
            );
            let actors = self.actors.read().await;

            let configs_with_manager = self.get_configs_with_manager_settings().await;

            if configs_with_manager.is_empty() {
                debug!(
                    log_type = LogType::ServerManager.as_str(),
                    "No servers with manager settings found"
                );
                return;
            }

            let mut server_counts: HashMap<String, usize> = HashMap::new();
            for (config_id, pairs) in actors.iter() {
                let active_count = pairs
                    .iter()
                    .filter(|pair| {
                        !pair.shutdown.load(std::sync::atomic::Ordering::SeqCst) && pair.is_login
                    })
                    .count();

                server_counts.insert(config_id.clone(), active_count);
                debug!(
                    log_type = LogType::ServerManager.as_str(),
                    "Server {} has {} active login connections", config_id, active_count
                );
            }

            for (config_id, manager_config) in configs_with_manager {
                let count = server_counts.get(&config_id).copied().unwrap_or(0);
                if count == 0 {
                    debug!(
                        log_type = LogType::ServerManager.as_str(),
                        "Server {} has no active connections", config_id
                    );
                    if let Some(empty_shutdown_time) = manager_config.empty_shutdown_time {
                        match server_manager
                            .get_status_for_server(
                                &manager_config.server_id,
                                manager_config.provider_name,
                            )
                            .await
                        {
                            Ok(status) => {
                                if status.state == infrarust_server_manager::ServerState::Running {
                                    debug!(
                                        log_type = LogType::ServerManager.as_str(),
                                        "Auto-shutdown enabled for {}@{:?} with timeout of {} seconds",
                                        config_id,
                                        manager_config.provider_name,
                                        empty_shutdown_time
                                    );

                                    if let Err(e) = server_manager
                                        .mark_server_as_empty(
                                            &manager_config.server_id,
                                            manager_config.provider_name,
                                            Duration::from_secs(empty_shutdown_time),
                                        )
                                        .await
                                    {
                                        debug!(
                                            log_type = LogType::ServerManager.as_str(),
                                            "Error marking server {} as empty: {}", config_id, e
                                        );
                                    } else {
                                        debug!(
                                            log_type = LogType::ServerManager.as_str(),
                                            "Server {} marked as empty, shutdown scheduled in {} seconds",
                                            config_id,
                                            empty_shutdown_time
                                        );
                                    }
                                } else {
                                    debug!(
                                        log_type = LogType::ServerManager.as_str(),
                                        "Server {} is not running (state: {:?}), not marking as empty",
                                        config_id,
                                        status.state
                                    );
                                }
                            }
                            Err(e) => {
                                debug!(
                                    log_type = LogType::ServerManager.as_str(),
                                    "Error getting status for server {}: {}", config_id, e
                                );
                            }
                        }
                    }
                } else {
                    debug!(
                        log_type = LogType::ServerManager.as_str(),
                        "Server {} has {} active connections, not marking as empty",
                        config_id,
                        count
                    );

                    let _ = server_manager
                        .remove_server_from_empty(
                            &manager_config.server_id,
                            manager_config.provider_name,
                        )
                        .await;
                }
            }
        }
    }

    pub(crate) async fn get_configs_with_manager_settings(&self) -> Vec<(String, ServerManagerConfig)> {
        let mut result = Vec::new();
        if let Some(shared) = crate::server::gateway::Gateway::get_shared_component() {
            let configs = shared
                .configuration_service()
                .get_all_configurations()
                .await;

            for (config_id, config) in configs {
                if let Some(manager_config) = &config.server_manager {
                    result.push((config_id.clone(), manager_config.clone()));
                }
            }
        }

        result
    }
}
