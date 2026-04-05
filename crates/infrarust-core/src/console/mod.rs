pub mod commands;
pub mod dispatcher;
pub mod output;
pub mod parser;

use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use infrarust_server_manager::ServerManagerService;

use crate::ban::manager::BanManager;
use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::plugin::manager::PluginManager;
use crate::registry::ConnectionRegistry;
use crate::services::config_service::ConfigServiceImpl;

pub struct ConsoleServices {
    pub player_registry: Arc<PlayerRegistryImpl>,
    pub connection_registry: Arc<ConnectionRegistry>,
    pub ban_manager: Arc<BanManager>,
    pub server_manager: Option<Arc<ServerManagerService>>,
    pub config_service: Arc<ConfigServiceImpl>,
    pub plugin_manager: Arc<tokio::sync::RwLock<PluginManager>>,
    pub permission_service: Arc<PermissionService>,
    pub shutdown: CancellationToken,
    pub start_time: Instant,
    is_tty: bool,
}

impl ConsoleServices {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        player_registry: Arc<PlayerRegistryImpl>,
        connection_registry: Arc<ConnectionRegistry>,
        ban_manager: Arc<BanManager>,
        server_manager: Option<Arc<ServerManagerService>>,
        config_service: Arc<ConfigServiceImpl>,
        plugin_manager: Arc<tokio::sync::RwLock<PluginManager>>,
        permission_service: Arc<PermissionService>,
        shutdown: CancellationToken,
        start_time: Instant,
    ) -> Self {
        Self {
            player_registry,
            connection_registry,
            ban_manager,
            server_manager,
            config_service,
            plugin_manager,
            permission_service,
            shutdown,
            start_time,
            is_tty: std::io::stdout().is_terminal(),
        }
    }

    pub fn is_tty(&self) -> bool {
        self.is_tty
    }
}

pub struct ConsoleTask {
    services: Arc<ConsoleServices>,
    dispatcher: dispatcher::CommandDispatcher,
    renderer: output::OutputRenderer,
}

impl ConsoleTask {
    pub fn new(services: Arc<ConsoleServices>) -> Self {
        let mut dispatcher = dispatcher::CommandDispatcher::new();
        commands::register_all(&mut dispatcher);
        let renderer = output::OutputRenderer::new();
        Self {
            services,
            dispatcher,
            renderer,
        }
    }

    pub async fn run(self) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);

        std::thread::Builder::new()
            .name("console-stdin".into())
            .spawn(move || {
                use std::io::BufRead;
                let stdin = std::io::stdin();
                let reader = std::io::BufReader::new(stdin.lock());
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            if tx.blocking_send(line).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();

        tracing::debug!(target: "console", "Console task started");

        loop {
            let line: String = tokio::select! {
                biased;
                () = self.services.shutdown.cancelled() => {
                    tracing::debug!(target: "console", "Console task shutting down");
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Some(line) => line,
                        None => {
                            tracing::info!(
                                target: "console",
                                "stdin closed (EOF), console disabled"
                            );
                            break;
                        }
                    }
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let output = self.dispatcher.dispatch(trimmed, &self.services).await;
            self.renderer.render(output).await;
        }
    }
}
