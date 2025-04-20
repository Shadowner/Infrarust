use crate::api::{ApiProvider, ApiServerStatus};
use crate::error::ServerManagerError;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum ServerState {
    Starting,
    Running,
    Stopping,
    Stopped,
    Crashed,
    Unknown,
}

impl From<&str> for ServerState {
    fn from(state: &str) -> Self {
        match state.to_lowercase().as_str() {
            "starting" => ServerState::Starting,
            "running" => ServerState::Running,
            "stopping" => ServerState::Stopping,
            "stopped" => ServerState::Stopped,
            "offline" => ServerState::Stopped,
            "crashed" => ServerState::Crashed,
            _ => ServerState::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub server_id: String,
    pub state: ServerState,
    pub last_checked: Instant,
    pub uptime: Option<Duration>,
    pub last_crash_time: Option<Instant>,
    pub crash_count: u32,
}

impl ServerStatus {
    pub fn new(server_id: &str) -> Self {
        ServerStatus {
            server_id: server_id.to_string(),
            state: ServerState::Unknown,
            last_checked: Instant::now(),
            uptime: None,
            last_crash_time: None,
            crash_count: 0,
        }
    }

    pub fn update_state(&mut self, new_state: ServerState) {
        if new_state == ServerState::Crashed && self.state != ServerState::Crashed {
            self.last_crash_time = Some(Instant::now());
            self.crash_count += 1;
        }

        if self.state != ServerState::Running && new_state == ServerState::Running {
            self.uptime = Some(Duration::from_secs(0));
        } else if self.state == ServerState::Running && new_state == ServerState::Running {
            if let Some(current_uptime) = self.uptime {
                let elapsed = self.last_checked.elapsed();
                self.uptime = Some(current_uptime + elapsed);
            }
        }

        self.state = new_state;
        self.last_checked = Instant::now();
    }

    pub fn is_crashed(&self) -> bool {
        self.state == ServerState::Crashed
    }

    pub fn is_running(&self) -> bool {
        self.state == ServerState::Running
    }
}

impl From<ApiServerStatus> for ServerStatus {
    fn from(api_status: ApiServerStatus) -> Self {
        let mut status = ServerStatus::new(&api_status.id);
        status.update_state(api_status.status);
        status
    }
}

pub async fn check_status<T: ApiProvider>(
    api: &T,
    server_id: &str,
) -> Result<ServerStatus, ServerManagerError> {
    let api_status = api.get_server_status(server_id).await?;

    let mut status = ServerStatus::from(api_status);
    status.server_id = server_id.to_string();

    Ok(status)
}
