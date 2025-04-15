use infrarust_config::models::server::ManagerType;
use infrarust_server_manager::{
    LocalProvider, PterodactylClient, ServerManager, ServerState, ServerStatus,
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, oneshot};
use tokio::time::sleep;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
struct ServerShutdownInfo {
    scheduled_at: Instant,
    shutdown_time: Duration,
}

type ShutdownTask = oneshot::Sender<()>;

#[derive(Debug, Clone)]
pub struct Manager {
    pterodactyl_manager: Arc<ServerManager<PterodactylClient>>,
    local_manager: Arc<ServerManager<LocalProvider>>,

    time_since_empty: Arc<Mutex<HashMap<ManagerType, HashMap<String, u64>>>>,
    shutdown_tasks: Arc<Mutex<HashMap<(ManagerType, String), ShutdownTask>>>,
    shutdown_timers: Arc<Mutex<HashMap<(ManagerType, String), ServerShutdownInfo>>>,
    starting_servers: Arc<Mutex<HashMap<(ManagerType, String), Instant>>>,
}

impl Manager {
    pub fn new(pterodactyl_client: PterodactylClient, local_client: LocalProvider) -> Self {
        let pterodactyl_manager = ServerManager::new(pterodactyl_client.clone());
        let local_manager = ServerManager::new(local_client.clone());

        // TODO: In the future
        // pterodactyl_manager.with_process_provider(pterodactyl_client);
        let local_manager = local_manager.with_process_provider(local_client);

        Self {
            pterodactyl_manager: Arc::new(pterodactyl_manager),
            local_manager: Arc::new(local_manager),
            time_since_empty: Arc::new(Mutex::new(HashMap::new())),
            shutdown_tasks: Arc::new(Mutex::new(HashMap::new())),
            shutdown_timers: Arc::new(Mutex::new(HashMap::new())),
            starting_servers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_status_for_server(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<ServerStatus, String> {
        //TODO: In the future let it being Dyn !
        match manager_type {
            ManagerType::Pterodactyl => {
                let status = self
                    .pterodactyl_manager
                    .get_server_status(server_id)
                    .await
                    .map_err(|e| e.to_string())?;

                // Update tracking state based on actual server state
                match status.state {
                    ServerState::Starting => {
                        debug!("Server {} is in starting state", server_id);
                        self.mark_server_as_starting(server_id, manager_type).await;
                    }
                    _ => {
                        debug!("Server {} is in state : {:?}", server_id, &manager_type);
                        self.remove_server_from_starting(server_id, &manager_type)
                            .await;
                    }
                }

                Ok(status)
            }
            ManagerType::Local => {
                let status = self
                    .local_manager
                    .get_server_status(server_id)
                    .await
                    .map_err(|e| e.to_string())?;

                // Update tracking state based on actual server state
                match status.state {
                    ServerState::Starting => {
                        debug!("Server {} is in starting state", server_id);
                        self.mark_server_as_starting(server_id, manager_type).await;
                    }
                    _ => {
                        debug!("Server {} is in state : {:?}", server_id, &manager_type);
                        self.remove_server_from_starting(server_id, &manager_type)
                            .await;
                    }
                }

                Ok(status)
            }
            _ => Err("Unsupported manager type".to_string()),
        }
    }

    pub async fn start_server(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<(), String> {
        debug!("Preparing to start server: {}", server_id);

        self.mark_server_as_starting(server_id, manager_type).await;

        if let Err(e) = self.remove_server_from_empty(server_id, manager_type).await {
            debug!("Error removing server from empty list: {}", e);
        }

        debug!("Starting server process: {}", server_id);
        match manager_type {
            ManagerType::Pterodactyl => self
                .pterodactyl_manager
                .start_server(server_id)
                .await
                .map_err(|e| e.to_string()),
            ManagerType::Local => self
                .local_manager
                .start_server(server_id)
                .await
                .map_err(|e| e.to_string()),
            _ => Err("Unsupported manager type".to_string()),
        }
    }

    pub async fn stop_server(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<(), String> {
        debug!("Stopping server: {}", server_id);

        //TODO: In the future let it being Dyn !
        let result = match manager_type {
            ManagerType::Pterodactyl => self
                .pterodactyl_manager
                .stop_server(server_id)
                .await
                .map_err(|e| e.to_string()),
            ManagerType::Local => self
                .local_manager
                .stop_server(server_id)
                .await
                .map_err(|e| e.to_string()),
            _ => Err("Unsupported manager type".to_string()),
        };

        self.remove_server_from_starting(server_id, &manager_type)
            .await;

        // Clean up all empty server tracking
        {
            let mut time_since_empty = self.time_since_empty.lock().await;
            if let Some(manager_map) = time_since_empty.get_mut(&manager_type) {
                manager_map.remove(server_id);
            }
        }

        // Clean up shutdown timers
        {
            let mut shutdown_timers = self.shutdown_timers.lock().await;
            shutdown_timers.remove(&(manager_type, server_id.to_string()));
        }

        // Cancel any shutdown tasks
        {
            let mut tasks = self.shutdown_tasks.lock().await;
            if let Some(tx) = tasks.remove(&(manager_type, server_id.to_string())) {
                let _ = tx.send(());
                debug!("Cancelled shutdown task for server: {}", server_id);
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

        match manager_type {
            ManagerType::Pterodactyl => self
                .pterodactyl_manager
                .restart_server(server_id)
                .await
                .map_err(|e| e.to_string()),
            ManagerType::Local => self
                .local_manager
                .restart_server(server_id)
                .await
                .map_err(|e| e.to_string()),
            _ => Err("Unsupported manager type".to_string()),
        }
    }

    pub async fn mark_server_as_starting(&self, server_id: &str, manager_type: ManagerType) {
        debug!("Marking server {} as starting", server_id);
        {
            let mut starting_servers = self.starting_servers.lock().await;
            starting_servers.insert((manager_type, server_id.to_string()), Instant::now());
        }
        debug!("Server {} marked as starting with timestamp", server_id);
    }

    pub async fn remove_server_from_starting(&self, server_id: &str, manager_type: &ManagerType) {
        debug!("Removing server {} from starting servers", server_id);
        // Use a separate scope to ensure the lock is released quickly
        {
            let mut starting_servers = self.starting_servers.lock().await;
            starting_servers.remove(&(*manager_type, server_id.to_string()));
        }
    }

    pub async fn is_server_starting(&self, server_id: &str, manager_type: &ManagerType) -> bool {
        // Get the result and release the lock immediately
        let key = (*manager_type, server_id.to_string());

        self.starting_servers.lock().await.contains_key(&key)
    }

    pub async fn mark_server_as_empty(
        &self,
        server_id: &str,
        manager_type: ManagerType,
        timeout: Duration,
    ) -> Result<(), String> {
        if self.is_server_starting(server_id, &manager_type).await {
            debug!(
                "Server {} is still starting, not marking as empty",
                server_id
            );
            return Ok(());
        }

        let already_marked_for_shutdown = {
            let shutdown_timers = self.shutdown_timers.lock().await;
            shutdown_timers.contains_key(&(manager_type, server_id.to_string()))
        };

        if already_marked_for_shutdown {
            debug!("Server {} is already marked for shutdown", server_id);
            return Ok(());
        }

        {
            let mut time_since_empty = self.time_since_empty.lock().await;
            let manager_map = time_since_empty
                .entry(manager_type)
                .or_insert_with(HashMap::new);
            manager_map.insert(server_id.to_string(), 0);
        }

        {
            let mut shutdown_timers = self.shutdown_timers.lock().await;
            shutdown_timers.insert(
                (manager_type, server_id.to_string()),
                ServerShutdownInfo {
                    scheduled_at: Instant::now(),
                    shutdown_time: timeout,
                },
            );
        }

        self.schedule_shutdown(server_id.to_string(), manager_type, timeout)
            .await;

        debug!("Marking server {} as empty", server_id);
        Ok(())
    }

    pub async fn remove_server_from_empty(
        &self,
        server_id: &str,
        manager_type: ManagerType,
    ) -> Result<(), String> {
        debug!("Removing server {} from empty", server_id);

        {
            let mut time_since_empty = self.time_since_empty.lock().await;
            if let Some(manager_map) = time_since_empty.get_mut(&manager_type) {
                manager_map.remove(server_id);
            }
        }

        {
            let mut shutdown_timers = self.shutdown_timers.lock().await;
            shutdown_timers.remove(&(manager_type, server_id.to_string()));
        }

        self.cancel_shutdown(server_id, &manager_type).await;
        Ok(())
    }

    pub async fn get_servers_near_shutdown(
        &self,
        threshold_seconds: u64,
    ) -> Vec<(String, ManagerType, u64)> {
        let shutdown_timers = self.shutdown_timers.lock().await;
        let now = Instant::now();
        let mut near_shutdown = Vec::new();

        for ((manager_type, server_id), info) in shutdown_timers.iter() {
            let elapsed = now.duration_since(info.scheduled_at);
            let remaining = if elapsed < info.shutdown_time {
                info.shutdown_time - elapsed
            } else {
                Duration::from_secs(0)
            };

            let remaining_secs = remaining.as_secs();
            if remaining_secs <= threshold_seconds {
                near_shutdown.push((server_id.clone(), *manager_type, remaining_secs));
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
        let key = (manager_type, server_id.clone());
        let should_create_new_task = {
            let tasks = self.shutdown_tasks.lock().await;
            !tasks.contains_key(&key)
        };

        if should_create_new_task {
            let (tx, mut rx) = oneshot::channel();

            {
                let mut tasks = self.shutdown_tasks.lock().await;
                tasks.insert(key, tx);
            }

            let manager_type_clone = manager_type;
            let server_id_clone = server_id.clone();
            let self_clone = Arc::new(self.clone());

            debug!(
                "Scheduling shutdown for {} in {} seconds",
                server_id,
                timeout.as_secs()
            );

            tokio::spawn(async move {
                tokio::select! {
                    _ = sleep(timeout) => {
                        // Double check if the server is still scheduled for shutdown
                        let shutdown_scheduled = {
                            let timers = self_clone.shutdown_timers.lock().await;
                            timers.contains_key(&(manager_type_clone, server_id_clone.clone()))
                        };

                        // Check again if the server is still starting before shutting down
                        if shutdown_scheduled && !self_clone.is_server_starting(&server_id_clone, &manager_type_clone).await {
                            debug!("From Shutdown Task : Auto-shutdown timer expired for empty server {}", server_id_clone);
                            match self_clone.stop_server(&server_id_clone, manager_type_clone).await {
                                Ok(_) => info!("Auto-shutting down empty server: {}", server_id_clone),
                                Err(e) => error!("From Shutdown Task : Failed to auto-shutdown server {}: {}", server_id_clone, e),
                            }
                        } else if self_clone.is_server_starting(&server_id_clone, &manager_type_clone).await {
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
                    tasks.remove(&(manager_type_clone, server_id_clone.clone()));
                }

                {
                    let mut shutdown_timers = self_clone.shutdown_timers.lock().await;
                    shutdown_timers.remove(&(manager_type_clone, server_id_clone.clone()));
                }
            });
        } else {
            debug!("Shutdown for {} is already scheduled", server_id);
        }
    }

    async fn cancel_shutdown(&self, server_id: &str, manager_type: &ManagerType) {
        let mut tasks = self.shutdown_tasks.lock().await;
        if let Some(tx) = tasks.remove(&(*manager_type, server_id.to_string())) {
            let _ = tx.send(());
        }
    }

    pub async fn force_clear_starting_status(&self, server_id: &str, manager_type: &ManagerType) {
        debug!("Force clearing starting status for server {}", server_id);
        let mut starting_servers = self.starting_servers.lock().await;
        if starting_servers
            .remove(&(*manager_type, server_id.to_string()))
            .is_some()
        {
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
