pub mod commands;
pub mod dispatcher;
pub mod output;
pub mod parser;
pub mod render;

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use infrarust_api::services::{ProxyRestart, ProxyUpdates};

use infrarust_server_manager::ServerManagerService;

use crate::ban::manager::BanManager;
use crate::permissions::PermissionService;
use crate::player::registry::PlayerRegistryImpl;
use crate::plugin::PluginManagerHandle;
use crate::registry::ConnectionRegistry;
use crate::services::command_manager::CommandManagerImpl;
use crate::services::config_service::ConfigServiceImpl;

pub struct ConsoleServices {
    pub player_registry: Arc<PlayerRegistryImpl>,
    pub connection_registry: Arc<ConnectionRegistry>,
    pub ban_manager: Arc<BanManager>,
    pub server_manager: Option<Arc<ServerManagerService>>,
    pub config_service: Arc<ConfigServiceImpl>,
    pub plugins: PluginManagerHandle,
    pub permission_service: Arc<PermissionService>,
    pub command_manager: Arc<CommandManagerImpl>,
    pub shutdown: CancellationToken,
    pub start_time: Instant,
    pub updates: Option<Arc<dyn ProxyUpdates>>,
    pub restart: Option<Arc<dyn ProxyRestart>>,
}

pub struct ConsoleTask {
    services: Arc<ConsoleServices>,
    dispatcher: dispatcher::CommandDispatcher,
}

impl ConsoleTask {
    pub fn new(services: Arc<ConsoleServices>) -> Self {
        let mut dispatcher = dispatcher::CommandDispatcher::new();
        commands::register_all(&mut dispatcher);
        Self {
            services,
            dispatcher,
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
            let rendered = render::Renderer::detect().render(&output);
            if !rendered.is_empty() {
                println!("{rendered}");
            }
        }
    }
}
