use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};
use tokio::sync::{mpsc, oneshot, RwLock};

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
    Connection,
};

#[derive(Clone)]
pub struct ActorPair {
    pub username: String,
    pub client: MinecraftClientHandler,
    pub server: MinecraftServerHandler,
    pub shutdown: Arc<AtomicBool>,
}

type ActorStorage = HashMap<String, Vec<ActorPair>>;

pub struct ActorSupervisor {
    actors: RwLock<ActorStorage>,
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
        }
    }

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

        match proxy_mode {
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
                .await
            }
        }
    }

    //TODO: Refactor this function to remove the clippy warning
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

        let client = MinecraftClientHandler::new(
            server_sender,
            client_receiver,
            client_handler,
            client_conn,
            is_login,
            username.clone(),
            shutdown_flag.clone(),
        );

        let server = MinecraftServerHandler::new(
            client_sender,
            server_receiver,
            is_login,
            oneshot_request_receiver,
            server_handler,
            shutdown_flag.clone(),
        );

        let pair = ActorPair {
            username: username.clone(),
            client,
            server,
            shutdown: shutdown_flag,
        };

        self.register_actor_pair(config_id, pair.clone()).await;
        pair
    }

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
                pair.shutdown
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            pairs.clear();
        }
    }
}
