use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use tokio::{
    sync::{OnceCell, RwLock, mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{Instrument, debug, debug_span, info, instrument};

use crate::{
    Connection,
    core::{
        actors::{client::MinecraftClientHandler, server::MinecraftServerHandler},
        config::ServerManagerConfig,
        event::MinecraftCommunication,
    },
    proxy_modes::{
        ClientProxyModeHandler, ProxyMessage, ProxyModeEnum, ServerProxyModeHandler,
        get_client_only_mode, get_offline_mode, get_passthrough_mode, get_status_mode,
    },
    server::{ServerResponse, manager::Manager},
};

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

pub enum SupervisorMessage {
    Shutdown,
    Disconnect,
}

#[derive(Clone, Debug)]
pub struct ActorPair {
    pub username: String,
    pub client: MinecraftClientHandler,
    pub server: MinecraftServerHandler,
    pub shutdown: Arc<AtomicBool>,
    pub created_at: std::time::Instant,
    pub session_id: uuid::Uuid,
    pub config_id: String,
    pub server_name: String,
    pub disconnect_logged: Arc<AtomicBool>,
    pub is_login: bool,
}

type ActorStorage = HashMap<String, Vec<ActorPair>>;

static GLOBAL_SUPERVISOR: OnceCell<Arc<ActorSupervisor>> = OnceCell::const_new();

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

#[derive(Debug)]
pub struct ActorSupervisor {
    actors: RwLock<ActorStorage>,
    tasks: RwLock<HashMap<String, Vec<JoinHandle<()>>>>,
    server_manager: Option<Arc<Manager>>,
}

impl Default for ActorSupervisor {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ActorSupervisor {
    pub fn global() -> Arc<ActorSupervisor> {
        match GLOBAL_SUPERVISOR.get() {
            Some(supervisor) => supervisor.clone(),
            None => {
                // Fallback to a new instance if not initialized (shouldn't happen in practice)
                debug!("Warning: Using temporary supervisor instance - global was not initialized");
                Arc::new(ActorSupervisor::new(None))
            }
        }
    }

    pub async fn get_task_statistics(&self) -> HashMap<String, TaskStats> {
        let tasks = self.tasks.read().await;
        let actors = self.actors.read().await;
        let mut stats = HashMap::new();

        for (config_id, handles) in tasks.iter() {
            let actor_count = actors.get(config_id).map_or(0, |pairs| {
                pairs
                    .iter()
                    .filter(|p| !p.shutdown.load(std::sync::atomic::Ordering::SeqCst))
                    .count()
            });

            let running_count = handles.iter().filter(|h| !h.is_finished()).count();

            let completed_count = handles.iter().filter(|h| h.is_finished()).count();

            let handles_info: Vec<TaskInfo> = handles
                .iter()
                .enumerate()
                .map(|(idx, handle)| TaskInfo {
                    id: idx,
                    is_finished: handle.is_finished(),
                    is_aborted: handle.is_finished(),
                })
                .collect();

            stats.insert(
                config_id.clone(),
                TaskStats {
                    config_id: config_id.clone(),
                    active_actor_count: actor_count,
                    task_count: handles.len(),
                    running_count,
                    completed_count,
                    orphaned_count: if actor_count == 0 { handles.len() } else { 0 },
                    task_handles: handles_info,
                },
            );
        }

        stats
    }

    pub fn initialize_global(
        server_manager: Option<Arc<Manager>>,
    ) -> Result<(), tokio::sync::SetError<Arc<ActorSupervisor>>> {
        debug!("Initializing global supervisor instance");
        GLOBAL_SUPERVISOR.set(Arc::new(ActorSupervisor::new(server_manager)))
    }

    pub fn new(server_manager: Option<Arc<Manager>>) -> Self {
        Self {
            actors: RwLock::new(HashMap::new()),
            tasks: RwLock::new(HashMap::new()),
            server_manager,
        }
    }

    // TODO : Refactor this to remove the allow
    #[allow(clippy::too_many_arguments)]
    #[instrument(name = "supervisor_create_pair", skip(self, client_conn, proxy_mode, oneshot_request_receiver), fields(
        config_id = %config_id,
        username = %username,
        proxy_mode = ?proxy_mode,
        is_login = is_login
    ))]
    pub async fn create_actor_pair(
        &self,
        config_id: &str,
        client_conn: Connection,
        proxy_mode: ProxyModeEnum,
        oneshot_request_receiver: oneshot::Receiver<ServerResponse>,
        is_login: bool,
        username: String,
        domain: &str,
    ) -> ActorPair {
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let span = debug_span!("actor_pair_setup");
        let session_id = client_conn.session_id;

        debug!(
            "Creating actor pair with session_id: {}, is_login: {}, proxy_mode: {:?}",
            session_id, is_login, proxy_mode
        );

        if is_login {
            #[cfg(feature = "telemetry")]
            TELEMETRY.update_player_count(1, config_id, client_conn.session_id, &username);
        }

        // TODO: Refactor this horror
        let pair = match proxy_mode {
            ProxyModeEnum::Status => {
                let (client_handler, server_handler) = get_status_mode();
                self.create_actor_pair_with_handlers(
                    config_id,
                    client_conn,
                    client_handler,
                    server_handler,
                    oneshot_request_receiver,
                    is_login,
                    username,
                    shutdown_flag,
                    session_id,
                    domain.to_string(),
                )
                .instrument(span)
                .await
            }
            ProxyModeEnum::Passthrough => {
                let (client_handler, server_handler) = get_passthrough_mode();
                self.create_actor_pair_with_handlers(
                    config_id,
                    client_conn,
                    client_handler,
                    server_handler,
                    oneshot_request_receiver,
                    is_login,
                    username,
                    shutdown_flag,
                    session_id,
                    domain.to_string(),
                )
                .instrument(span)
                .await
            }
            ProxyModeEnum::Offline => {
                let (client_handler, server_handler) = get_offline_mode();
                self.create_actor_pair_with_handlers(
                    config_id,
                    client_conn,
                    client_handler,
                    server_handler,
                    oneshot_request_receiver,
                    is_login,
                    username,
                    shutdown_flag,
                    session_id,
                    domain.to_string(),
                )
                .instrument(span)
                .await
            }
            ProxyModeEnum::ClientOnly => {
                let (client_handler, server_handler) = get_client_only_mode();
                self.create_actor_pair_with_handlers(
                    config_id,
                    client_conn,
                    client_handler,
                    server_handler,
                    oneshot_request_receiver,
                    is_login,
                    username,
                    shutdown_flag,
                    session_id,
                    domain.to_string(),
                )
                .instrument(span)
                .await
            }
            ProxyModeEnum::ServerOnly => {
                let (client_handler, server_handler) = get_passthrough_mode();
                self.create_actor_pair_with_handlers(
                    config_id,
                    client_conn,
                    client_handler,
                    server_handler,
                    oneshot_request_receiver,
                    is_login,
                    username,
                    shutdown_flag,
                    session_id,
                    domain.to_string(),
                )
                .instrument(span)
                .await
            }
        };

        self.register_actor_pair(config_id, pair.clone())
            .instrument(debug_span!("register_pair"))
            .await;

        debug!("Actor pair created successfully");
        pair
    }

    #[instrument(skip(self, client_conn, client_handler, server_handler, oneshot_request_receiver, shutdown_flag), fields(
        config_id = %config_id,
        username = %username,
        is_login = is_login
    ))]
    #[allow(clippy::too_many_arguments)]
    async fn create_actor_pair_with_handlers<T>(
        &self,
        config_id: &str,
        client_conn: Connection,
        client_handler: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
        server_handler: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
        oneshot_request_receiver: oneshot::Receiver<ServerResponse>,
        is_login: bool,
        username: String,
        shutdown_flag: Arc<AtomicBool>,
        session_id: uuid::Uuid,
        server_name: String,
    ) -> ActorPair
    where
        T: ProxyMessage + 'static + Send + Sync + std::fmt::Debug,
    {
        let (server_sender, server_receiver) = mpsc::channel(64);
        let (client_sender, client_receiver) = mpsc::channel(64);

        let root_span = if is_login {
            Some(debug_span!(
                parent: None,
                "actor_handling",
                username = %username,
                is_login = is_login
            ))
        } else {
            None
        };

        let client = MinecraftClientHandler::new(
            server_sender,
            client_receiver,
            client_handler,
            client_conn,
            is_login,
            username.clone(),
            shutdown_flag.clone(),
            root_span.clone(),
        )
        .await;

        let server = MinecraftServerHandler::new(
            client_sender,
            server_receiver,
            is_login,
            oneshot_request_receiver,
            server_handler,
            shutdown_flag.clone(),
            root_span.clone(),
        );

        ActorPair {
            username: username.clone(),
            client,
            server,
            shutdown: shutdown_flag,
            created_at: std::time::Instant::now(),
            session_id,
            config_id: config_id.to_string(),
            server_name,
            disconnect_logged: Arc::new(AtomicBool::new(false)),
            is_login,
        }
    }

    async fn log_disconnect_if_needed(&self, pair: &ActorPair) {
        if !pair
            .disconnect_logged
            .load(std::sync::atomic::Ordering::SeqCst)
            && !pair.username.is_empty()
            && pair.created_at.elapsed().as_secs() > 5
        // Only log meaningful connections
        {
            info!(
                "Player '{}' disconnected from server '{}' ({})",
                pair.username, pair.server_name, pair.config_id
            );

            let duration_secs = pair.created_at.elapsed().as_secs();
            debug!(
                "Session duration for '{}': {} seconds",
                pair.username, duration_secs
            );
            pair.disconnect_logged
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[instrument(skip(self, pair), fields(config_id = %config_id))]
    async fn register_actor_pair(&self, config_id: &str, pair: ActorPair) {
        let mut actors = self.actors.write().await;
        actors
            .entry(config_id.to_string())
            .or_insert_with(Vec::new)
            .push(pair);
    }

    pub async fn shutdown_actors(&self, config_id: &str) {
        let mut actors = self.actors.write().await;
        if let Some(pairs) = actors.get_mut(config_id) {
            for pair in pairs.iter() {
                debug!("Shutting down actor for user {}", pair.username);
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
                    "Cleaned up {} dead actors for config {}",
                    before_count - after_count,
                    config_id
                );

                // Clean up any associated tasks
                if let Some(task_handles) = tasks.get_mut(config_id) {
                    while task_handles.len() > pairs.len() {
                        if let Some(handle) = task_handles.pop() {
                            debug!("Aborting orphaned task for {}", config_id);
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
                    debug!("Aborting orphaned task for {}", config_id);
                    handle.abort();
                }
                false
            } else {
                true
            }
        });
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
                                    "Player '{}' disconnected from server '{}' ({}) - reason: {}",
                                    pair.username, pair.server_name, config_id, reason
                                );

                                let duration_secs = pair.created_at.elapsed().as_secs();
                                debug!(
                                    "Session duration for '{}': {} seconds",
                                    pair.username, duration_secs
                                );

                                pair.disconnect_logged
                                    .store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                        } else {
                            // For non-login sessions (status requests), just debug log
                            debug!(
                                "Status Request connection disconnected from server '{}' ({}) - reason: {}",
                                pair.server_name, config_id, reason
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
            let mut tasks = self.tasks.write().await;

            for config_id in config_ids_to_clean {
                if let Some(task_handles) = tasks.get_mut(&config_id) {
                    let actors_count = {
                        let actors = self.actors.read().await;
                        actors.get(&config_id).map_or(0, |pairs| pairs.len())
                    };

                    while task_handles.len() > actors_count {
                        if let Some(handle) = task_handles.pop() {
                            debug!("Aborting task for disconnected session in {}", config_id);
                            handle.abort();
                        }
                    }
                }
            }
        }

        #[cfg(feature = "telemetry")]
        for (session_id, config_id) in actors_to_remove {
            TELEMETRY.update_player_count(-1, &config_id, session_id, "");
        }

        self.check_and_mark_empty_servers().await;
        debug!("Cleanup completed for session {}", session_id);
    }

    pub async fn find_actor_pairs_by_session_id(
        &self,
        session_id: uuid::Uuid,
    ) -> Option<Vec<Arc<RwLock<ActorPair>>>> {
        let actors = self.actors.read().await;
        let mut result = Vec::new();

        for pairs in actors.values() {
            for pair in pairs {
                if pair.session_id == session_id {
                    let pair_clone = Arc::new(RwLock::new(pair.clone()));
                    result.push(pair_clone);
                }
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Get all active actors, used by CLI commands
    pub async fn get_all_actors(&self) -> HashMap<String, Vec<ActorPair>> {
        let actors = self.actors.read().await;
        let mut result = HashMap::new();

        for (config_id, pairs) in actors.iter() {
            // Only include pairs that aren't shut down
            let active_pairs: Vec<ActorPair> = pairs
                .iter()
                .filter(|pair| !pair.shutdown.load(std::sync::atomic::Ordering::SeqCst))
                .cloned()
                .collect();

            if !active_pairs.is_empty() {
                result.insert(config_id.clone(), active_pairs);
            }
        }

        result
    }

    /// Shutdown all actors across all servers
    pub async fn shutdown_all_actors(&self) {
        info!("Shutting down all actors");
        let mut actors = self.actors.write().await;

        for (config_id, pairs) in actors.iter_mut() {
            for pair in pairs.iter() {
                debug!(
                    "Shutting down actor for user {} on {}",
                    pair.username, config_id
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
            debug!("Aborting {} tasks for {}", handles.len(), config_id);
            for handle in handles.iter() {
                handle.abort();
            }
        }
        tasks.clear();

        info!("All actors have been shut down");
    }

    pub async fn check_and_mark_empty_servers(&self) {
        if let Some(server_manager) = &self.server_manager {
            debug!("Checking for empty servers");
            let actors = self.actors.read().await;

            let configs_with_manager = self.get_configs_with_manager_settings().await;

            if configs_with_manager.is_empty() {
                debug!("No servers with manager settings found");
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
                    "Server {} has {} active login connections",
                    config_id, active_count
                );
            }

            for (config_id, manager_config) in configs_with_manager {
                let count = server_counts.get(&config_id).copied().unwrap_or(0);
                if count == 0 {
                    debug!("Server {} has no active connections", config_id);
                    if let Some(empty_shutdown_time) = manager_config.empty_shutdown_time {
                        match server_manager
                            .get_status_for_server(
                                &manager_config.server_id,
                                manager_config.provider_name.clone(),
                            )
                            .await
                        {
                            Ok(status) => {
                                if status.state == infrarust_server_manager::ServerState::Running {
                                    debug!(
                                        "Auto-shutdown enabled for {}@{:?} with timeout of {} seconds",
                                        config_id,
                                        manager_config.provider_name,
                                        empty_shutdown_time
                                    );

                                    if let Err(e) = server_manager
                                        .mark_server_as_empty(
                                            &manager_config.server_id,
                                            manager_config.provider_name.clone(),
                                            Duration::from_secs(empty_shutdown_time),
                                        )
                                        .await
                                    {
                                        debug!(
                                            "Error marking server {} as empty: {}",
                                            config_id, e
                                        );
                                    } else {
                                        debug!(
                                            "Server {} marked as empty, shutdown scheduled in {} seconds",
                                            config_id, empty_shutdown_time
                                        );
                                    }
                                } else {
                                    debug!(
                                        "Server {} is not running (state: {:?}), not marking as empty",
                                        config_id, status.state
                                    );
                                }
                            }
                            Err(e) => {
                                debug!("Error getting status for server {}: {}", config_id, e);
                            }
                        }
                    }
                } else {
                    debug!(
                        "Server {} has {} active connections, not marking as empty",
                        config_id, count
                    );

                    let _ = server_manager
                        .remove_server_from_empty(
                            &manager_config.server_id,
                            manager_config.provider_name.clone(),
                        )
                        .await;
                }
            }
        }
    }

    async fn get_configs_with_manager_settings(&self) -> Vec<(String, ServerManagerConfig)> {
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
