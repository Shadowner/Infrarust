use async_trait::async_trait;
use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;
use tracing::debug;

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
    pub _handle: Arc<JoinHandle<Result<(), ServerManagerError>>>,
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
        let processes = self.processes.lock().unwrap();
        if let Some(process) = processes.get(server_id) {
            let state = process.server_state.lock().unwrap();
            return Ok(state.clone());
        }
        drop(processes);

        let server_states = self.server_states.lock().unwrap();
        if let Some(state) = server_states.get(server_id) {
            return Ok(state.clone());
        }

        Ok(ServerState::Stopped)
    }

    pub fn start_process(
        &self,
        server_id: &str,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        startup_string: Option<&str>,
    ) -> Result<ProcessOutput, ServerManagerError> {
        {
            let processes = self.processes.lock().unwrap();
            if processes.contains_key(server_id) {
                return Err(ServerManagerError::ProcessError(format!(
                    "Process for server {} is already running",
                    server_id
                )));
            }
        }

        let (stdout_tx, _) = broadcast::channel::<String>(100); // Using broadcast instead of mpsc
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(100);

        let (caller_tx, caller_rx) = mpsc::channel::<String>(100);

        let stdout_tx_clone = stdout_tx.clone();
        let caller_tx_clone = caller_tx.clone();
        let server_id_string = server_id.to_string();
        let startup_string_clone = startup_string.map(|s| s.to_string());
        {
            let mut server_states = self.server_states.lock().unwrap();
            server_states.insert(server_id.to_string(), ServerState::Starting);
            debug!(
                "Set server state for '{}' to Starting in server_states map",
                server_id
            );
        }
        let server_state = Arc::new(Mutex::new(ServerState::Starting));
        debug!(
            "Created server_state Arc with initial value Starting for '{}'",
            server_id
        );

        let mut command_builder = Command::new(command);
        command_builder
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(dir) = working_dir {
            command_builder.current_dir(dir);
        }

        let mut child = match command_builder.spawn() {
            Ok(child) => child,
            Err(e) => {
                // Update state to Stopped on spawn failure
                debug!("Failed to spawn process for '{}': {}", server_id, e);
                let mut server_states = self.server_states.lock().unwrap();
                server_states.insert(server_id.to_string(), ServerState::Stopped);
                debug!(
                    "Set server state for '{}' back to Stopped due to spawn failure",
                    server_id
                );

                return Err(ServerManagerError::ProcessError(format!(
                    "Failed to start process: {}",
                    e
                )));
            }
        };

        debug!("Process for '{}' spawned successfully", server_id);

        let mut stdin = child.stdin.take().expect("Failed to open stdin");
        let stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");

        let server_state_clone = server_state.clone();
        let stdout_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buffer = [0; 1024];
            let mut started = false;

            loop {
                match std::io::Read::read(&mut reader, &mut buffer) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        let output = String::from_utf8_lossy(&buffer[0..n]).to_string();

                        if !started {
                            if let Some(ref startup_str) = startup_string_clone {
                                if output.contains(startup_str) {
                                    // Server has started
                                    let mut state = server_state_clone.lock().unwrap();
                                    *state = ServerState::Running;
                                    debug!(
                                        "Detected startup string, set server_state to Running for '{}'",
                                        server_id_string
                                    );
                                    started = true;
                                }
                            }
                        }

                        let _ = stdout_tx_clone.send(output.clone());
                        if (caller_tx_clone.send(output).await).is_err() {}
                    }
                    Err(e) => {
                        eprintln!("Error reading stdout: {}", e);
                        break;
                    }
                }
            }
            debug!("stdout reader task for '{}' exited", server_id_string);
            Ok::<(), ServerManagerError>(())
        });

        let stdout_tx_stderr = stdout_tx.clone();
        let caller_tx_stderr = caller_tx.clone();
        let server_id_stderr = server_id.to_string();

        // Read stderr and redirect to the same channel
        let stderr_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0; 1024];

            loop {
                match std::io::Read::read(&mut reader, &mut buffer) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        let output = String::from_utf8_lossy(&buffer[0..n]).to_string();
                        // Send to both the broadcast channel and the initial caller's channel
                        let _ = stdout_tx_stderr.send(output.clone());
                        if (caller_tx_stderr.send(output).await).is_err() {
                            // The caller's receiver was dropped, but that's okay
                        }
                    }
                    Err(e) => {
                        eprintln!("Error reading stderr: {}", e);
                        break;
                    }
                }
            }
            debug!("stderr reader task for '{}' exited", server_id_stderr);
            Ok::<(), ServerManagerError>(())
        });

        let server_id_stdin = server_id.to_string();
        let stdin_handle = tokio::spawn(async move {
            while let Some(input) = stdin_rx.recv().await {
                if let Err(e) = stdin.write_all(input.as_bytes()) {
                    eprintln!("Failed to write to stdin: {}", e);
                    break;
                }
                if let Err(e) = stdin.flush() {
                    eprintln!("Failed to flush stdin: {}", e);
                    break;
                }
            }
            debug!("stdin writer task for '{}' exited", server_id_stdin);
            Ok::<(), ServerManagerError>(())
        });

        let server_id_clone = server_id.to_string();
        let processes_clone = self.processes.clone();
        let server_state_clone = server_state.clone();
        let server_states_clone = self.server_states.clone();

        let handle = tokio::spawn(async move {
            debug!("Process monitor task started for '{}'", server_id_clone);

            debug!(
                "Process '{}' maintaining Starting state after spawn",
                server_id_clone
            );

            let timeout_duration = tokio::time::Duration::from_secs(5);
            debug!(
                "Setting up timeout of {:?} for '{}' startup",
                timeout_duration, server_id_clone
            );
            tokio::time::sleep(timeout_duration).await;

            debug!(
                "Process '{}' exited, setting state to Stopped",
                server_id_clone
            );

            let _ = stdout_handle.await;
            let _ = stderr_handle.await;
            let _ = stdin_handle.await;

            {
                debug!(
                    "Updating process state for '{}' to Stopped after exit",
                    server_id_clone
                );
                let mut state = server_state_clone.lock().unwrap();
                *state = ServerState::Stopped;

                let mut states = server_states_clone.lock().unwrap();
                debug!(
                    "Updating server_states map for '{}' to Stopped after exit",
                    server_id_clone
                );
                states.insert(server_id_clone.clone(), ServerState::Stopped);
            }

            {
                debug!(
                    "Removing process '{}' from processes map after exit",
                    server_id_clone
                );
                let mut processes = processes_clone.lock().unwrap();
                processes.remove(&server_id_clone);
            }

            debug!(
                "Process monitor task for '{}' completed successfully",
                server_id_clone
            );
            Ok(())
        });

        let process = ManagedProcess {
            _server_id: server_id.to_string(),
            stdout_tx,
            stdin_tx,
            _handle: Arc::new(handle),
            server_state,
        };

        {
            let mut processes = self.processes.lock().unwrap();
            processes.insert(server_id.to_string(), process);
            debug!("Added process '{}' to processes map", server_id);
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
            let processes = self.processes.lock().unwrap();
            match processes.get(server_id) {
                Some(process) => process.stdin_tx.clone(),
                None => {
                    return Err(ServerManagerError::ProcessError(format!(
                        "No process found for server {}",
                        server_id
                    )));
                }
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
        let processes = self.processes.lock().unwrap();
        match processes.get(server_id) {
            Some(process) => {
                let (tx, rx) = mpsc::channel::<String>(100);
                let mut broadcast_rx = process.stdout_tx.subscribe();

                // Spawn a task to forward messages from the broadcast to the mpsc channel
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
    }

    fn is_process_running(&self, server_id: &str) -> Result<bool, ServerManagerError> {
        let processes = self.processes.lock().unwrap();
        let process_exists = processes.contains_key(server_id);

        debug!(
            "is_process_running check for '{}': exists in map: {}",
            server_id, process_exists
        );

        if !process_exists {
            debug!("Process '{}' not found in processes map", server_id);
            return Ok(false);
        }

        let (handle_arc, server_state_arc) = if let Some(process) = processes.get(server_id) {
            (process._handle.clone(), process.server_state.clone())
        } else {
            return Ok(false);
        };

        drop(processes);

        let finished = handle_arc.is_finished();
        debug!("Process '{}' task is_finished: {}", server_id, finished);

        if finished {
            debug!(
                "Process '{}' confirmed terminated via handle.is_finished()",
                server_id
            );
            // Use the helper function to handle the terminated process
            handle_terminated_process(self, server_id);
            return Ok(false);
        }

        let server_state = { server_state_arc.lock().unwrap().clone() };

        if server_state == ServerState::Stopped {
            debug!(
                "Process '{}' marked as Stopped in internal state, considering terminated",
                server_id
            );
            handle_terminated_process(self, server_id);
            return Ok(false);
        }

        // This is a Linux-specific check that can detect zombie processes
        #[cfg(target_os = "linux")]
        {
            let _process_id: Option<u32> = {
                let processes = self.processes.lock().unwrap();
                if let Some(_process) = processes.get(server_id) {
                    // We don't store the OS Process ID currently, so we can't check
                    // Adding this framework for future implementation
                    None
                } else {
                    None
                }
            };
        }

        debug!("Process '{}' is confirmed running", server_id);
        Ok(true)
    }

    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError> {
        {
            let mut server_states = self.server_states.lock().unwrap();
            server_states.insert(server_id.to_string(), ServerState::Stopped);
        }

        {
            let mut processes = self.processes.lock().unwrap();
            if let Some(process) = processes.get(server_id) {
                let mut state = process.server_state.lock().unwrap();
                *state = ServerState::Stopped;
            }

            if !processes.contains_key(server_id) {
                return Err(ServerManagerError::ProcessError(format!(
                    "No process found for server {}",
                    server_id
                )));
            }

            processes.remove(server_id);
        }

        Ok(())
    }
}

fn handle_terminated_process(pm: &ProcessManager, server_id: &str) {
    debug!("Process '{}' has terminated, updating state", server_id);
    {
        let mut states = pm.server_states.lock().unwrap();
        states.insert(server_id.to_string(), ServerState::Stopped);
        debug!("Updated server_states map for '{}' to Stopped", server_id);
    }

    {
        let mut processes = pm.processes.lock().unwrap();
        if let Some(process) = processes.get(server_id) {
            let mut state = process.server_state.lock().unwrap();
            *state = ServerState::Stopped;
            debug!(
                "Updated process internal state for '{}' to Stopped",
                server_id
            );
        }
        processes.remove(server_id);
        debug!("Removed process '{}' from processes map", server_id);
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}
