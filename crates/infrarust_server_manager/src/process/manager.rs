use async_trait::async_trait;
use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;
use tracing::{debug, error};

use crate::error::ServerManagerError;
use crate::monitor::ServerState;
use crate::process::provider::ProcessProvider;

#[derive(Clone, Debug)]
pub struct ProcessManager {
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
    server_states: Arc<Mutex<HashMap<String, ServerState>>>,
}

#[derive(Clone, Debug)]
pub struct ManagedProcess {
    pub _server_id: String,
    pub stdout_tx: broadcast::Sender<String>,
    pub stdin_tx: Sender<String>,
    pub handle: Arc<JoinHandle<Result<(), ServerManagerError>>>,
    pub server_state: Arc<Mutex<ServerState>>,
}

pub struct ProcessOutput {
    pub server_id: String,
    pub stdout_rx: Receiver<String>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            server_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_server_state(&self, server_id: &str) -> Result<ServerState, ServerManagerError> {
        // Check running processes first
        if let Ok(processes) = self.processes.lock()
            && let Some(process) = processes.get(server_id)
            && let Ok(state) = process.server_state.lock()
        {
            return Ok(state.clone());
        }

        // Fall back to server states map
        if let Ok(server_states) = self.server_states.lock()
            && let Some(state) = server_states.get(server_id)
        {
            return Ok(state.clone());
        }

        Ok(ServerState::Stopped)
    }

    fn set_server_state(&self, server_id: &str, state: ServerState) {
        // Update both the process state and the global state map
        if let Ok(processes) = self.processes.lock()
            && let Some(process) = processes.get(server_id)
            && let Ok(mut process_state) = process.server_state.lock()
        {
            *process_state = state.clone();
            debug!(
                log_type = "server_manager",
                "Updated process state for '{}' to {:?}", server_id, state
            );
        }

        if let Ok(mut server_states) = self.server_states.lock() {
            server_states.insert(server_id.to_string(), state.clone());
            debug!(
                log_type = "server_manager",
                "Updated global state for '{}' to {:?}", server_id, state
            );
        }
    }

    fn cleanup_process(&self, server_id: &str) {
        debug!(
            log_type = "server_manager",
            "Cleaning up process '{}'", server_id
        );

        if let Ok(mut processes) = self.processes.lock() {
            processes.remove(server_id);
            debug!(
                log_type = "server_manager",
                "Removed process '{}' from processes map", server_id
            );
        }

        self.set_server_state(server_id, ServerState::Stopped);
    }

    pub fn start_process(
        &self,
        server_id: &str,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        startup_string: Option<&str>,
    ) -> Result<ProcessOutput, ServerManagerError> {
        // Check if process already exists
        if let Ok(processes) = self.processes.lock()
            && processes.contains_key(server_id)
        {
            return Err(ServerManagerError::ProcessError(format!(
                "Process for server {} is already running",
                server_id
            )));
        }

        // Create channels
        let (stdout_tx, _) = broadcast::channel::<String>(100);
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(100);
        let (caller_tx, caller_rx) = mpsc::channel::<String>(100);

        // Set initial state
        self.set_server_state(server_id, ServerState::Starting);

        // Build and spawn command
        let mut command_builder = Command::new(command);
        command_builder
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(dir) = working_dir {
            command_builder.current_dir(dir);
        }

        let mut child = command_builder.spawn().map_err(|e| {
            self.set_server_state(server_id, ServerState::Stopped);
            ServerManagerError::ProcessError(format!("Failed to start process: {}", e))
        })?;

        debug!(
            log_type = "server_manager",
            "Process for '{}' spawned successfully", server_id
        );

        // Take stdio handles
        let mut stdin = child.stdin.take().expect("Failed to open stdin");
        let stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");

        // Create shared state
        let server_state = Arc::new(Mutex::new(ServerState::Starting));
        let server_id_string = server_id.to_string();
        let startup_string_clone = startup_string.map(|s| s.to_string());

        // Stdout reader task
        let stdout_tx_clone = stdout_tx.clone();
        let caller_tx_clone = caller_tx.clone();
        let server_state_clone = server_state.clone();
        let server_id_stdout = server_id_string.clone();

        let stdout_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buffer = [0; 1024];
            let mut started = false;

            loop {
                match std::io::Read::read(&mut reader, &mut buffer) {
                    Ok(0) => {
                        debug!(
                            log_type = "server_manager",
                            "stdout EOF for '{}'", server_id_stdout
                        );
                        break;
                    }
                    Ok(n) => {
                        let output = String::from_utf8_lossy(&buffer[0..n]).to_string();

                        // Check for startup string
                        if !started
                            && let Some(ref startup_str) = startup_string_clone
                            && output.contains(startup_str)
                            && let Ok(mut state) = server_state_clone.lock()
                        {
                            *state = ServerState::Running;
                            debug!("Server '{}' started successfully", server_id_stdout);
                            started = true;
                        }

                        // Send output to channels (ignore errors if receivers are dropped)
                        let _ = stdout_tx_clone.send(output.clone());
                        let _ = caller_tx_clone.send(output).await;
                    }
                    Err(e) => {
                        error!(
                            log_type = "server_manager",
                            "Error reading stdout for '{}': {}", server_id_stdout, e
                        );
                        break;
                    }
                }
            }
            debug!(
                log_type = "server_manager",
                "stdout reader task for '{}' exited", server_id_stdout
            );
        });

        // Stderr reader task
        let stdout_tx_stderr = stdout_tx.clone();
        let caller_tx_stderr = caller_tx.clone();
        let server_id_stderr = server_id_string.clone();

        let stderr_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0; 1024];

            loop {
                match std::io::Read::read(&mut reader, &mut buffer) {
                    Ok(0) => {
                        debug!(
                            log_type = "server_manager",
                            "stderr EOF for '{}'", server_id_stderr
                        );
                        break;
                    }
                    Ok(n) => {
                        let output = String::from_utf8_lossy(&buffer[0..n]).to_string();
                        let _ = stdout_tx_stderr.send(output.clone());
                        let _ = caller_tx_stderr.send(output).await;
                    }
                    Err(e) => {
                        error!(
                            log_type = "server_manager",
                            "Error reading stderr for '{}': {}", server_id_stderr, e
                        );
                        break;
                    }
                }
            }
            debug!(
                log_type = "server_manager",
                "stderr reader task for '{}' exited", server_id_stderr
            );
        });

        // Stdin writer task
        let server_id_stdin = server_id_string.clone();
        let stdin_handle = tokio::spawn(async move {
            while let Some(input) = stdin_rx.recv().await {
                if let Err(e) = stdin.write_all(input.as_bytes()) {
                    error!(
                        log_type = "server_manager",
                        "Failed to write to stdin for '{}': {}", server_id_stdin, e
                    );
                    break;
                }
                if let Err(e) = stdin.flush() {
                    error!(
                        log_type = "server_manager",
                        "Failed to flush stdin for '{}': {}", server_id_stdin, e
                    );
                    break;
                }
            }
            debug!(
                log_type = "server_manager",
                "stdin writer task for '{}' exited", server_id_stdin
            );
        });

        // Main process monitor task
        let processes_clone = self.processes.clone();
        let server_states_clone = self.server_states.clone();
        let server_state_clone = server_state.clone();
        let server_id_monitor = server_id_string.clone();

        let handle = tokio::spawn(async move {
            debug!(
                log_type = "server_manager",
                "Process monitor started for '{}'", server_id_monitor
            );

            // Wait for the child process to exit
            let exit_status = child.wait();
            debug!(
                "Process '{}' exited with status: {:?}",
                server_id_monitor, exit_status
            );

            // Don't wait for the IO tasks - just abort them to prevent hanging
            stdout_handle.abort();
            stderr_handle.abort();
            stdin_handle.abort();

            // Update states immediately
            if let Ok(mut state) = server_state_clone.lock() {
                *state = ServerState::Stopped;
            }

            if let Ok(mut states) = server_states_clone.lock() {
                states.insert(server_id_monitor.clone(), ServerState::Stopped);
            }

            // Remove from processes map
            if let Ok(mut processes) = processes_clone.lock() {
                processes.remove(&server_id_monitor);
            }

            debug!(
                log_type = "server_manager",
                "Process monitor for '{}' completed", server_id_monitor
            );
            Ok(())
        });

        // Create managed process
        let process = ManagedProcess {
            _server_id: server_id_string,
            stdout_tx,
            stdin_tx,
            handle: Arc::new(handle),
            server_state,
        };

        // Add to processes map
        if let Ok(mut processes) = self.processes.lock() {
            processes.insert(server_id.to_string(), process);
            debug!(
                log_type = "server_manager",
                "Added process '{}' to processes map", server_id
            );
        }

        Ok(ProcessOutput {
            server_id: server_id.to_string(),
            stdout_rx: caller_rx,
        })
    }
}

#[async_trait]
impl ProcessProvider for ProcessManager {
    async fn write_stdin(&self, server_id: &str, input: &str) -> Result<(), ServerManagerError> {
        let stdin_tx = {
            if let Ok(processes) = self.processes.lock() {
                match processes.get(server_id) {
                    Some(process) => process.stdin_tx.clone(),
                    None => {
                        return Err(ServerManagerError::ProcessError(format!(
                            "No process found for server {}",
                            server_id
                        )));
                    }
                }
            } else {
                return Err(ServerManagerError::ProcessError(
                    "Failed to access processes map".to_string(),
                ));
            }
        };

        let input = if input.ends_with('\n') {
            input.to_string()
        } else {
            format!("{}\n", input)
        };

        stdin_tx.send(input).await.map_err(|e| {
            ServerManagerError::ProcessError(format!("Failed to write to stdin: {}", e))
        })?;

        Ok(())
    }

    fn get_stdout_stream(&self, server_id: &str) -> Result<Receiver<String>, ServerManagerError> {
        if let Ok(processes) = self.processes.lock() {
            match processes.get(server_id) {
                Some(process) => {
                    let (tx, rx) = mpsc::channel::<String>(100);
                    let mut broadcast_rx = process.stdout_tx.subscribe();

                    tokio::spawn(async move {
                        while let Ok(msg) = broadcast_rx.recv().await {
                            if tx.send(msg).await.is_err() {
                                break;
                            }
                        }
                    });

                    Ok(rx)
                }
                None => Err(ServerManagerError::ProcessError(format!(
                    "No process found for server {}",
                    server_id
                ))),
            }
        } else {
            Err(ServerManagerError::ProcessError(
                "Failed to access processes map".to_string(),
            ))
        }
    }

    fn is_process_running(&self, server_id: &str) -> Result<bool, ServerManagerError> {
        if let Ok(processes) = self.processes.lock()
            && let Some(process) = processes.get(server_id)
        {
            // Check if the monitor task is still running
            if process.handle.is_finished() {
                debug!(
                    log_type = "server_manager",
                    "Process '{}' monitor task finished, cleaning up", server_id
                );
                drop(processes);
                self.cleanup_process(server_id);
                return Ok(false);
            }

            // Check the internal state
            if let Ok(state) = process.server_state.lock() {
                let is_running = *state != ServerState::Stopped;
                debug!(
                    "Process '{}' state check: {:?} (running: {})",
                    server_id, *state, is_running
                );
                return Ok(is_running);
            }
        }

        debug!(
            log_type = "server_manager",
            "Process '{}' not found or not running", server_id
        );
        Ok(false)
    }

    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError> {
        debug!("Stopping process '{}'", server_id);

        // Get the handle to abort the monitor task
        let handle_arc = {
            if let Ok(processes) = self.processes.lock() {
                processes.get(server_id).map(|p| p.handle.clone())
            } else {
                None
            }
        };

        // Abort the monitor task if it exists
        if let Some(handle) = handle_arc {
            handle.abort();
            debug!("Aborted monitor task for '{}'", server_id);
        }

        // Clean up the process
        self.cleanup_process(server_id);

        Ok(())
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}
