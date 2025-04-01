use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};
use tokio::{
    sync::{mpsc, oneshot, OnceCell, RwLock},
    task::JoinHandle,
};
use tracing::{debug, debug_span, info, instrument, Instrument};

use crate::{
    core::{
        actors::{client::MinecraftClientHandler, server::MinecraftServerHandler},
        event::MinecraftCommunication,
    },
    proxy_modes::{
        get_client_only_mode, get_offline_mode, get_passthrough_mode, get_status_mode,
        ClientProxyModeHandler, ProxyMessage, ProxyModeEnum, ServerProxyModeHandler,
    },
    server::ServerResponse,
    telemetry::TELEMETRY,
    Connection,
};

pub enum SupervisorMessage {
    Shutdown,
    Disconnect,
}

#[derive(Clone)]
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
}

type ActorStorage = HashMap<String, Vec<ActorPair>>;

static GLOBAL_SUPERVISOR: OnceCell<Arc<ActorSupervisor>> = OnceCell::const_new();

pub struct ActorSupervisor {
    actors: RwLock<ActorStorage>,
    tasks: RwLock<HashMap<String, Vec<JoinHandle<()>>>>,
}

impl Default for ActorSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl ActorSupervisor {
    pub fn global() -> Arc<ActorSupervisor> {
        match GLOBAL_SUPERVISOR.get() {
            Some(supervisor) => supervisor.clone(),
            None => {
                // Fallback to a new instance if not initialized (shouldn't happen in practice)
                debug!("Warning: Using temporary supervisor instance - global was not initialized");
                Arc::new(ActorSupervisor::new())
            }
        }
    }

    pub fn initialize_global(supervisor: Arc<ActorSupervisor>) -> Result<(), tokio::sync::SetError<Arc<ActorSupervisor>>> {
        debug!("Initializing global supervisor instance");
        GLOBAL_SUPERVISOR.set(supervisor)
    }

    pub fn new() -> Self {
        Self {
            actors: RwLock::new(HashMap::new()),
            tasks: RwLock::new(HashMap::new()),
        }
    }

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
        domain:&str
    ) -> ActorPair {
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let span = debug_span!("actor_pair_setup");
        let session_id = client_conn.session_id;

        debug!(
            "Creating actor pair with session_id: {}, is_login: {}, proxy_mode: {:?}",
            session_id, is_login, proxy_mode
        );

        if is_login {
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
        ).await;

        let server = MinecraftServerHandler::new(
            client_sender,
            server_receiver,
            is_login,
            oneshot_request_receiver,
            server_handler,
            shutdown_flag.clone(),
            root_span.clone(),
        );

        let pair = ActorPair {
            username: username.clone(),
            client,
            server,
            shutdown: shutdown_flag,
            created_at: std::time::Instant::now(),
            session_id,
            config_id: config_id.to_string(),
            server_name,
            disconnect_logged: Arc::new(AtomicBool::new(false)),
        };

        pair
    }

    async fn log_disconnect_if_needed(&self, pair: &ActorPair) {
        if !pair.disconnect_logged.load(std::sync::atomic::Ordering::SeqCst) 
            && !pair.username.is_empty() 
            && pair.created_at.elapsed().as_secs() > 5 // Only log meaningful connections
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
            pair.disconnect_logged.store(true, std::sync::atomic::Ordering::SeqCst);
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
    
    pub async fn log_player_disconnect(&self, session_id: uuid::Uuid, reason: &str) {
        let actors = self.actors.read().await;
        // Find the actor pair with this session ID
        for (config_id, pairs) in actors.iter() {
            for pair in pairs {
                if pair.session_id == session_id && !pair.username.is_empty() {
                    // Only log if not already logged
                    if !pair.disconnect_logged.load(std::sync::atomic::Ordering::SeqCst) {
                        info!(
                            "Player '{}' disconnected from server '{}' ({}) - reason: {}",
                            pair.username, pair.server_name, config_id, reason
                        );
                        
                        // Mark as logged to avoid duplicates
                        pair.disconnect_logged.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    
                    // Mark this pair for shutdown
                    pair.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
            }
        }
    }

    pub async fn find_actor_pairs_by_session_id(&self, session_id: uuid::Uuid) -> Option<Vec<Arc<RwLock<ActorPair>>>> {
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
                debug!("Shutting down actor for user {} on {}", pair.username, config_id);
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
}
