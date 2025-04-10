use async_trait::async_trait;
use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;

use crate::error::ServerManagerError;
use crate::process::provider::ProcessProvider;

#[derive(Clone, Debug)]
pub struct ProcessManager {
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
}

#[derive(Clone, Debug)]
pub struct ManagedProcess {
    pub _server_id: String,
    pub stdout_tx: broadcast::Sender<String>,
    pub stdin_tx: Sender<String>,
    pub _handle: Arc<JoinHandle<Result<(), ServerManagerError>>>,
}

pub struct ProcessOutput {
    pub server_id: String,
    pub stdout_rx: Receiver<String>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_process(
        &self,
        server_id: &str,
        command: &str,
        args: &[&str],
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
        let _server_id_string = server_id.to_string();

        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                ServerManagerError::ProcessError(format!("Failed to start process: {}", e))
            })?;

        let mut stdin = child.stdin.take().expect("Failed to open stdin");
        let stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");

        let stdout_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buffer = [0; 1024];

            loop {
                match std::io::Read::read(&mut reader, &mut buffer) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        let output = String::from_utf8_lossy(&buffer[0..n]).to_string();
                        // Send to both the broadcast channel and the initial caller's channel
                        let _ = stdout_tx_clone.send(output.clone());
                        if (caller_tx_clone.send(output).await).is_err() {
                            // The caller's receiver was dropped, but that's okay
                        }
                    }
                    Err(e) => {
                        eprintln!("Error reading stdout: {}", e);
                        break;
                    }
                }
            }
            Ok::<(), ServerManagerError>(())
        });

        let stdout_tx_stderr = stdout_tx.clone();
        let caller_tx_stderr = caller_tx.clone();

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
            Ok::<(), ServerManagerError>(())
        });

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
            Ok::<(), ServerManagerError>(())
        });

        let server_id_clone = server_id.to_string();
        let processes_clone = self.processes.clone();

        let handle = tokio::spawn(async move {
            let status = child.wait().map_err(|e| {
                ServerManagerError::ProcessError(format!("Failed to wait for process: {}", e))
            })?;

            let _ = stdout_handle.await;
            let _ = stderr_handle.await;
            let _ = stdin_handle.await;

            let mut processes = processes_clone.lock().unwrap();
            processes.remove(&server_id_clone);

            if !status.success() {
                return Err(ServerManagerError::ProcessError(format!(
                    "Process for server {} exited with status: {}",
                    server_id_clone, status
                )));
            }

            Ok(())
        });

        let process = ManagedProcess {
            _server_id: server_id.to_string(),
            stdout_tx,
            stdin_tx,
            _handle: Arc::new(handle),
        };

        {
            let mut processes = self.processes.lock().unwrap();
            processes.insert(server_id.to_string(), process);
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
        Ok(self.processes.lock().unwrap().contains_key(server_id))
    }

    async fn stop_process(&self, server_id: &str) -> Result<(), ServerManagerError> {
        {
            let mut processes = self.processes.lock().unwrap();
            match processes.remove(server_id) {
                Some(process) => process,
                None => {
                    return Err(ServerManagerError::ProcessError(format!(
                        "No process found for server {}",
                        server_id
                    )));
                }
            }
        };

        Ok(())
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}
