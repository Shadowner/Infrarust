use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};
use tokio::{sync::{mpsc, oneshot, RwLock}, task::JoinHandle};
use tracing::{debug, debug_span, instrument, Instrument};

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
}

type ActorStorage = HashMap<String, Vec<ActorPair>>;

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
    ) -> ActorPair {
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let span = debug_span!("actor_pair_setup");

        if is_login {
            TELEMETRY.update_player_count(1, config_id, client_conn.session_id, &username);
        }

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
                )
                .instrument(span)
                .await
            }
        };

        self.register_actor_pair(config_id, pair.clone())
            .instrument(debug_span!("register_pair"))
            .await;

        pair
    }

    #[instrument(skip(self, client_conn, client_handler, server_handler, oneshot_request_receiver, shutdown_flag), fields(
        config_id = %config_id,
        username = %username,
        is_login = is_login
    ))]
    //TODO: Refactor to remove the warning
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
    ) -> ActorPair
    where
        T: ProxyMessage + 'static + Send + Sync,
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
        );

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
        };

        self.register_actor_pair(config_id, pair.clone()).await;
        pair
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
                pair.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
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
        tasks.entry(config_id.to_string())
            .or_insert_with(Vec::new)
            .push(handle);
    }
    
    // Periodic health check to clear dead actors
    // TODO: Upgrade to a more sophisticated actor lifecycle management system
    pub async fn health_check(&self) {
        let mut actors = self.actors.write().await;
        let mut tasks = self.tasks.write().await;
        
        for (config_id, pairs) in actors.iter_mut() {
            let before_count = pairs.len();
            
            // Remove actors with shutdown flag set
            pairs.retain(|pair| !pair.shutdown.load(std::sync::atomic::Ordering::SeqCst));
            
            let after_count = pairs.len();
            if before_count != after_count {
                debug!("Cleaned up {} dead actors for config {}", before_count - after_count, config_id);
                
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
}
