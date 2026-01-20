use std::sync::{Arc, atomic::AtomicBool};

use infrarust_config::{LogType, models::server::ProxyModeEnum};
use tokio::sync::mpsc;
use tracing::{Instrument, debug, debug_span, instrument};

use crate::{
    Connection,
    core::{
        actors::{
            client::{ClientHandlerConfig, MinecraftClientHandler},
            server::MinecraftServerHandler,
        },
        event::MinecraftCommunication,
    },
    proxy_modes::{ClientProxyModeHandler, ProxyMessage, ServerProxyModeHandler},
    server::ServerResponse,
};

use super::ActorSupervisor;

/// A pair of client and server actors for handling a Minecraft connection
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

impl ActorSupervisor {
    #[allow(clippy::too_many_arguments)] // Use builder() for cleaner API
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
        oneshot_request_receiver: tokio::sync::oneshot::Receiver<ServerResponse>,
        is_login: bool,
        username: String,
        domain: &str,
    ) -> ActorPair {
        use crate::proxy_modes::{
            get_client_only_mode, get_offline_mode, get_passthrough_mode, get_status_mode,
        };

        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let span = debug_span!("actor_pair_setup", log_type = LogType::Supervisor.as_str());
        let session_id = client_conn.session_id;

        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Creating actor pair with session_id: {}, is_login: {}, proxy_mode: {:?}",
            session_id,
            is_login,
            proxy_mode
        );

        if is_login {
            #[cfg(feature = "telemetry")]
            crate::telemetry::TELEMETRY.update_player_count(
                1,
                config_id,
                client_conn.session_id,
                &username,
            );
        }

        // Macro to reduce boilerplate for creating actor pairs with different handler types
        macro_rules! create_pair_with_mode {
            ($get_mode:expr) => {{
                let (client_handler, server_handler) = $get_mode;
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
            }};
        }

        let pair = match proxy_mode {
            ProxyModeEnum::Status => create_pair_with_mode!(get_status_mode()),
            ProxyModeEnum::Passthrough | ProxyModeEnum::ServerOnly => {
                create_pair_with_mode!(get_passthrough_mode())
            }
            ProxyModeEnum::Offline => create_pair_with_mode!(get_offline_mode()),
            ProxyModeEnum::ClientOnly => create_pair_with_mode!(get_client_only_mode()),
        };

        self.register_actor_pair(config_id, pair.clone())
            .instrument(debug_span!("register_pair"))
            .await;

        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Actor pair created successfully"
        );
        pair
    }

    #[instrument(skip(self, client_conn, client_handler, server_handler, oneshot_request_receiver, shutdown_flag), fields(
        config_id = %config_id,
        username = %username,
        is_login = is_login
    ))]
    #[allow(clippy::too_many_arguments)] // Internal generic method - use builder() for public API
    pub(crate) async fn create_actor_pair_with_handlers<T>(
        &self,
        config_id: &str,
        client_conn: Connection,
        client_handler: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
        server_handler: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
        oneshot_request_receiver: tokio::sync::oneshot::Receiver<ServerResponse>,
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

        let client = MinecraftClientHandler::new(ClientHandlerConfig {
            server_sender,
            client_receiver,
            proxy_mode: client_handler,
            conn: client_conn,
            is_login,
            username: username.clone(),
            shutdown: shutdown_flag.clone(),
            start_span: root_span.clone(),
        })
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

    #[instrument(skip(self, pair), fields(config_id = %config_id))]
    pub(crate) async fn register_actor_pair(&self, config_id: &str, pair: ActorPair) {
        let mut actors = self.actors.write().await;
        actors
            .entry(config_id.to_string())
            .or_insert_with(Vec::new)
            .push(pair);
    }
}
