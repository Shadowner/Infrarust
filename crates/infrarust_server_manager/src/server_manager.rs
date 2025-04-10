use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time;

pub use crate::ServerState;
use crate::api::ApiProvider;
use crate::error::ServerManagerError;
pub use crate::monitor::{CrashDetector, ServerStatus};
use crate::terminal::execute_command;

pub struct ServerManager<T: ApiProvider> {
    api_client: Arc<T>,
    crash_detector: CrashDetector,
    status_check_interval: Duration,
}

impl<T: ApiProvider> ServerManager<T> {
    pub fn new(api_client: T) -> Self {
        Self {
            api_client: Arc::new(api_client),
            crash_detector: CrashDetector::default(),
            status_check_interval: Duration::from_secs(30), 
        }
    }

    pub fn with_check_interval(mut self, interval: Duration) -> Self {
        self.status_check_interval = interval;
        self
    }

    pub fn with_crash_detector(mut self, detector: CrashDetector) -> Self {
        self.crash_detector = detector;
        self
    }

    pub async fn monitor_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let server_id = server_id.to_string();
        let api_client = Arc::clone(&self.api_client);
        let status = Arc::new(Mutex::new(ServerStatus::new(&server_id)));

        println!("Starting to monitor server: {}", server_id);

        loop {
            match api_client.get_server_status(&server_id).await {
                Ok(api_status) => {
                    let mut status_lock = status.lock().await;

                    if api_status.is_running {
                        status_lock.update_state(ServerState::Running);
                    } else if api_status.is_crashed {
                        status_lock.update_state(ServerState::Crashed);
                    } else {
                        status_lock.update_state(ServerState::Stopped);
                    }

                    if self.crash_detector.is_in_crash_loop(&status_lock) {
                        println!("ALERT: Server {} is in a crash loop!", server_id);
                    }

                    println!("Server {} status: {:?}", server_id, status_lock.state);
                }
                Err(e) => {
                    println!("Error checking server status: {}", e);
                }
            }

            time::sleep(self.status_check_interval).await;
        }
    }

    pub async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        println!("Starting server: {}", server_id);
        self.api_client.start_server(server_id).await
    }

    pub async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        println!("Stopping server: {}", server_id);
        self.api_client.stop_server(server_id).await
    }

    pub async fn restart_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        println!("Restarting server: {}", server_id);
        self.api_client.restart_server(server_id).await
    }

    pub fn execute_system_command(&self, command: &str) -> Result<String, ServerManagerError> {
        println!("Executing command: {}", command);
        execute_command(command)
    }

    pub async fn get_server_status(&self, server_id: &str) -> Result<ServerStatus, ServerManagerError> {
        let status = self.api_client.get_server_status(server_id).await?;
        Ok(status.into())
    }
}
