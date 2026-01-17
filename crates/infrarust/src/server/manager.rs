use infrarust_config::models::{logging::LogType, server::ManagerType};
use infrarust_server_manager::{
    CraftyClient, LocalProvider, ManagerDispatcher, PterodactylClient, ServerManager, ServerState,
    ServerStatus,
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock, oneshot};
use tokio::time::sleep;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
struct ServerShutdownInfo {
    scheduled_at: Instant,
    shutdown_time: Duration,
}

type ShutdownTask = oneshot::Sender<()>;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ServerKey {
    manager_type: ManagerType,
    server_id: String,
}

impl ServerKey {
    pub fn new(manager_type: ManagerType, server_id: impl Into<String>) -> Self {
        Self {
            manager_type,
            server_id: server_id.into(),
        }
    }
}

#[derive(Clone)]
pub struct Manager {
    dispatchers: Arc<HashMap<ManagerType, Arc<dyn ManagerDispatcher>>>,
    local_manager: Arc<ServerManager<LocalProvider>>,

    time_since_empty: Arc<RwLock<HashMap<ManagerType, HashMap<String, u64>>>>,
    shutdown_tasks: Arc<Mutex<HashMap<ServerKey, ShutdownTask>>>,
    shutdown_timers: Arc<RwLock<HashMap<ServerKey, ServerShutdownInfo>>>,
    starting_servers: Arc<RwLock<HashMap<ServerKey, Instant>>>,
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manager")
            .field("dispatchers", &format!("{} manager types", self.dispatchers.len()))
            .field("local_manager", &self.local_manager)
            .finish_non_exhaustive()
    }
}

impl Manager {
    pub fn new(
        pterodactyl_client: PterodactylClient,
        local_client: LocalProvider,
        crafty_client: CraftyClient,
    ) -> Self {
        let pterodactyl_manager = Arc::new(ServerManager::new(pterodactyl_client));
        let local_manager = ServerManager::new(local_client.clone());
        let crafty_manager = Arc::new(ServerManager::new(crafty_client));
        let local_manager = Arc::new(local_manager.with_process_provider(local_client));

        // Build dispatcher map for dynamic dispatch
        let mut dispatchers: HashMap<ManagerType, Arc<dyn ManagerDispatcher>> = HashMap::new();
        dispatchers.insert(
            ManagerType::Pterodactyl,
            pterodactyl_manager as Arc<dyn ManagerDispatcher>,
        );
        dispatchers.insert(
            ManagerType::Local,
            local_manager.clone() as Arc<dyn ManagerDispatcher>,
        );
        dispatchers.insert(
            ManagerType::Crafty,
            crafty_manager as Arc<dyn ManagerDispatcher>,
        );

        Self {
            dispatchers: Arc::new(dispatchers),
            local_manager,
            time_since_empty: Arc::new(RwLock::new(HashMap::new())),
            shutdown_tasks: Arc::new(Mutex::new(HashMap::new())),
            shutdown_timers: Arc::new(RwLock::new(HashMap::new())),
            starting_servers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn get_dispatcher(&self, manager_type: ManagerType) -> Result<&Arc<dyn ManagerDispatcher>, String> {
        self.dispatchers
            .get(&manager_type)
            .ok_or_else(|| format!("Unsupported manager type: {:?}", manager_type))
    }

    pub async fn get_status_for_server(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<ServerStatus, String> {
        let dispatcher = self.get_dispatcher(manager_type)?;
        let status = dispatcher
            .get_status(server_id)
            .await
            .map_err(|e| e.to_string())?;

        // Update tracking state based on actual server state
        match status.state {
            ServerState::Starting => {
                debug!(
                    log_type = LogType::ServerManager.as_str(),
                    "Server {} is in starting state", server_id
                );
                self.mark_server_as_starting(server_id, manager_type).await;
            }
            _ => {
                debug!(
                    log_type = LogType::ServerManager.as_str(),
                    "Server {} is in state: {:?}", server_id, status.state
                );
                self.remove_server_from_starting(server_id, manager_type)
                    .await;
            }
        }

        Ok(status)
    }

    pub async fn start_server(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<(), String> {
        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Preparing to start server: {}", server_id
        );

        self.mark_server_as_starting(server_id, manager_type).await;

        if let Err(e) = self.remove_server_from_empty(server_id, manager_type).await {
            debug!(
                log_type = LogType::ServerManager.as_str(),
                "Error removing server from empty list: {}", e
            );
        }

        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Starting server process: {}", server_id
        );

        let dispatcher = self.get_dispatcher(manager_type)?;
        dispatcher
            .start(server_id)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn stop_server(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<(), String> {
        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Stopping server: {}", server_id
        );

        let dispatcher = self.get_dispatcher(manager_type)?;
        let result = dispatcher.stop(server_id).await.map_err(|e| e.to_string());

        self.remove_server_from_starting(server_id, manager_type)
            .await;

        let key = ServerKey::new(manager_type, server_id);

        // Clean up all empty server tracking
        {
            let mut time_since_empty = self.time_since_empty.write().await;
            if let Some(manager_map) = time_since_empty.get_mut(&manager_type) {
                manager_map.remove(server_id);
            }
        }

        // Clean up shutdown timers
        {
            let mut shutdown_timers = self.shutdown_timers.write().await;
            shutdown_timers.remove(&key);
        }

        // Cancel any shutdown tasks
        {
            let mut tasks = self.shutdown_tasks.lock().await;
            if let Some(tx) = tasks.remove(&key) {
                let _ = tx.send(());
                debug!(
                    log_type = LogType::ServerManager.as_str(),
                    "Cancelled shutdown task for server: {}", server_id
                );
            }
        }

        result
    }

    pub async fn restart_server(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<(), String> {
        self.mark_server_as_starting(server_id, manager_type).await;
        self.remove_server_from_empty(server_id, manager_type)
            .await?;

        let dispatcher = self.get_dispatcher(manager_type)?;
        dispatcher
            .restart(server_id)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn mark_server_as_starting(&self, server_id: &str, manager_type: ManagerType) {
        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Marking server {} as starting", server_id
        );
        let key = ServerKey::new(manager_type, server_id);
        {
            let mut starting_servers = self.starting_servers.write().await;
            starting_servers.insert(key, Instant::now());
        }
        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Server {} marked as starting with timestamp", server_id
        );
    }

    pub async fn remove_server_from_starting(&self, server_id: &str, manager_type: ManagerType) {
        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Removing server {} from starting servers", server_id
        );
        let key = ServerKey::new(manager_type, server_id);
        {
            let mut starting_servers = self.starting_servers.write().await;
            starting_servers.remove(&key);
        }
    }

    pub async fn is_server_starting(&self, server_id: &str, manager_type: ManagerType) -> bool {
        let key = ServerKey::new(manager_type, server_id);
        self.starting_servers.read().await.contains_key(&key)
    }

    pub async fn mark_server_as_empty(
        &self,
        server_id: &str,
        manager_type: ManagerType,
        timeout: Duration,
    ) -> Result<(), String> {
        if self.is_server_starting(server_id, manager_type).await {
            debug!(
                log_type = LogType::ServerManager.as_str(),
                "Server {} is still starting, not marking as empty", server_id
            );
            return Ok(());
        }

        let key = ServerKey::new(manager_type, server_id);

        let already_marked_for_shutdown = {
            let shutdown_timers = self.shutdown_timers.read().await;
            shutdown_timers.contains_key(&key)
        };

        if already_marked_for_shutdown {
            debug!(
                log_type = LogType::ServerManager.as_str(),
                "Server {} is already marked for shutdown", server_id
            );
            return Ok(());
        }

        {
            let mut time_since_empty = self.time_since_empty.write().await;
            let manager_map = time_since_empty
                .entry(manager_type)
                .or_default();
            manager_map.insert(server_id.to_string(), 0);
        }

        {
            let mut shutdown_timers = self.shutdown_timers.write().await;
            shutdown_timers.insert(
                key,
                ServerShutdownInfo {
                    scheduled_at: Instant::now(),
                    shutdown_time: timeout,
                },
            );
        }

        self.schedule_shutdown(server_id.to_string(), manager_type, timeout)
            .await;

        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Marking server {} as empty", server_id
        );
        Ok(())
    }

    pub async fn remove_server_from_empty(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<(), String> {
        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Removing server {} from empty", server_id
        );

        let key = ServerKey::new(manager_type, server_id);

        {
            let mut time_since_empty = self.time_since_empty.write().await;
            if let Some(manager_map) = time_since_empty.get_mut(&manager_type) {
                manager_map.remove(server_id);
            }
        }

        {
            let mut shutdown_timers = self.shutdown_timers.write().await;
            shutdown_timers.remove(&key);
        }

        self.cancel_shutdown(server_id, manager_type).await;
        Ok(())
    }

    pub async fn get_servers_near_shutdown(
        &self,
        threshold_seconds: u64,
    ) -> Vec<(String, ManagerType, u64)> {
        let shutdown_timers = self.shutdown_timers.read().await;
        let now = Instant::now();
        let mut near_shutdown = Vec::new();

        for (key, info) in shutdown_timers.iter() {
            let elapsed = now.duration_since(info.scheduled_at);
            let remaining = if elapsed < info.shutdown_time {
                info.shutdown_time - elapsed
            } else {
                Duration::from_secs(0)
            };

            let remaining_secs = remaining.as_secs();
            if remaining_secs <= threshold_seconds {
                near_shutdown.push((key.server_id.clone(), key.manager_type, remaining_secs));
            }
        }

        near_shutdown
    }

    async fn schedule_shutdown(
        &self,
        server_id: String,
        manager_type: ManagerType,
        timeout: Duration,
    ) {
        let key = ServerKey::new(manager_type, &server_id);
        let should_create_new_task = {
            let tasks = self.shutdown_tasks.lock().await;
            !tasks.contains_key(&key)
        };

        if should_create_new_task {
            let (tx, mut rx) = oneshot::channel();

            {
                let mut tasks = self.shutdown_tasks.lock().await;
                tasks.insert(key.clone(), tx);
            }

            let server_id_clone = server_id.clone();
            let self_clone = Arc::new(self.clone());

            debug!(
                log_type = LogType::ServerManager.as_str(),
                "Scheduling shutdown for {} in {} seconds",
                server_id,
                timeout.as_secs()
            );

            tokio::spawn(async move {
                let task_key = ServerKey::new(manager_type, &server_id_clone);

                tokio::select! {
                    _ = sleep(timeout) => {
                        // Double check if the server is still scheduled for shutdown
                        let shutdown_scheduled = {
                            let timers = self_clone.shutdown_timers.read().await;
                            timers.contains_key(&task_key)
                        };

                        // Check again if the server is still starting before shutting down
                        if shutdown_scheduled && !self_clone.is_server_starting(&server_id_clone, manager_type).await {
                            debug!(log_type = LogType::ServerManager.as_str(), "From Shutdown Task : Auto-shutdown timer expired for empty server {}", server_id_clone);
                            match self_clone.stop_server(&server_id_clone, manager_type).await {
                                Ok(_) => info!("Auto-shutting down empty server: {}", server_id_clone),
                                Err(e) => error!("From Shutdown Task : Failed to auto-shutdown server {}: {}", server_id_clone, e),
                            }
                        } else if self_clone.is_server_starting(&server_id_clone, manager_type).await {
                            debug!("From Shutdown Task : Server {} is still starting, cancel auto-shutdown", server_id_clone);
                        } else {
                            debug!("From Shutdown Task : Shutdown for {} was canceled before timer expired", server_id_clone);
                        }
                    }
                    _ = &mut rx => {
                        debug!("From Shutdown Task : Auto-shutdown explicitly cancelled for server: {}", server_id_clone);
                    }
                }

                {
                    let mut tasks = self_clone.shutdown_tasks.lock().await;
                    tasks.remove(&task_key);
                }

                {
                    let mut shutdown_timers = self_clone.shutdown_timers.write().await;
                    shutdown_timers.remove(&task_key);
                }
            });
        } else {
            debug!("Shutdown for {} is already scheduled", server_id);
        }
    }

    async fn cancel_shutdown(&self, server_id: &str, manager_type: ManagerType) {
        let key = ServerKey::new(manager_type, server_id);
        let mut tasks = self.shutdown_tasks.lock().await;
        if let Some(tx) = tasks.remove(&key) {
            let _ = tx.send(());
        }
    }

    pub async fn force_clear_starting_status(&self, server_id: &str, manager_type: ManagerType) {
        debug!("Force clearing starting status for server {}", server_id);
        let key = ServerKey::new(manager_type, server_id);
        let mut starting_servers = self.starting_servers.write().await;
        if starting_servers.remove(&key).is_some() {
            debug!(
                "Removed server {} from starting servers (forced)",
                server_id
            );
        }
    }

    pub(crate) fn local_provider(&self) -> Arc<ServerManager<LocalProvider>> {
        self.local_manager.clone()
    }
}
