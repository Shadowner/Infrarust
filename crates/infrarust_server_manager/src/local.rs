use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use infrarust_config::LocalManagerConfig;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::task::JoinHandle;

use crate::error::ServerManagerError;
use crate::provider::{ProviderFuture, ProviderStatus, ServerProvider};

pub struct LocalProvider {
    config: LocalManagerConfig,
    process: tokio::sync::Mutex<Option<LocalProcess>>,
}

struct LocalProcess {
    child: Child,
    stdin: ChildStdin,
    ready: Arc<AtomicBool>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
}

impl LocalProvider {
    pub fn new(config: LocalManagerConfig) -> Self {
        Self {
            config,
            process: tokio::sync::Mutex::new(None),
        }
    }

    fn server_label(&self) -> String {
        format!(
            "{}:{}",
            self.config.command,
            self.config.working_dir.display()
        )
    }
}

impl ServerProvider for LocalProvider {
    fn start(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            let mut process_lock = self.process.lock().await;
            if process_lock.is_some() {
                return Err(std::io::Error::other("process is already running").into());
            }

            let mut cmd = tokio::process::Command::new(&self.config.command);
            cmd.args(&self.config.args)
                .current_dir(&self.config.working_dir)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);

            let mut child = cmd.spawn().map_err(ServerManagerError::from)?;

            let stdin = child.stdin.take().ok_or_else(|| {
                ServerManagerError::from(std::io::Error::other("stdin not available"))
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                ServerManagerError::from(std::io::Error::other("stdout not available"))
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                ServerManagerError::from(std::io::Error::other("stderr not available"))
            })?;

            let ready = Arc::new(AtomicBool::new(false));
            let ready_clone = Arc::clone(&ready);
            let pattern = self.config.ready_pattern.clone();
            let label = self.server_label();

            let stdout_task = tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(server = %label, "{line}");
                    if line.contains(&pattern) {
                        tracing::info!(server = %label, "ready pattern detected");
                        ready_clone.store(true, Ordering::Release);
                    }
                }
            });

            let label2 = self.server_label();
            let stderr_task = tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(server = %label2, "[stderr] {line}");
                }
            });

            *process_lock = Some(LocalProcess {
                child,
                stdin,
                ready,
                stdout_task,
                stderr_task,
            });
            drop(process_lock);

            tracing::info!(server = %self.server_label(), "process spawned");
            Ok(())
        })
    }

    fn stop(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            let mut process_lock = self.process.lock().await;
            let process = process_lock.as_mut().ok_or_else(|| {
                ServerManagerError::from(std::io::Error::other("process is not running"))
            })?;

            if let Err(e) = process.stdin.write_all(b"stop\n").await {
                tracing::warn!(server = %self.server_label(), "failed to write stop command: {e}");
            }
            let _ = process.stdin.flush().await;

            let shutdown_timeout = self.config.shutdown_timeout;
            match tokio::time::timeout(shutdown_timeout, process.child.wait()).await {
                Ok(Ok(status)) => {
                    tracing::info!(server = %self.server_label(), "process exited with {status}");
                }
                Ok(Err(e)) => {
                    tracing::warn!(server = %self.server_label(), "error waiting for process: {e}");
                }
                Err(_) => {
                    tracing::warn!(server = %self.server_label(), "shutdown timeout, killing process");
                    let _ = process.child.kill().await;
                }
            }

            process.stdout_task.abort();
            process.stderr_task.abort();
            *process_lock = None;
            drop(process_lock);

            Ok(())
        })
    }

    fn check_status(&self) -> ProviderFuture<'_, ProviderStatus> {
        Box::pin(async move {
            let mut process_lock = self.process.lock().await;
            let result = match process_lock.as_mut() {
                None => Ok(ProviderStatus::Stopped),
                Some(process) => match process.child.try_wait() {
                    Ok(Some(status)) => {
                        process.stdout_task.abort();
                        process.stderr_task.abort();
                        *process_lock = None;
                        if status.success() {
                            Ok(ProviderStatus::Stopped)
                        } else {
                            Ok(ProviderStatus::Crashed)
                        }
                    }
                    Ok(None) | Err(_) => {
                        if process.ready.load(Ordering::Acquire) {
                            Ok(ProviderStatus::Running)
                        } else {
                            Ok(ProviderStatus::Starting)
                        }
                    }
                },
            };
            drop(process_lock);
            result
        })
    }

    fn provider_type(&self) -> &'static str {
        "local"
    }

    fn owns_child_process(&self) -> bool {
        true
    }
}
