//! Shutdown coordination for graceful termination

use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, info};

#[derive(Debug)]
pub struct ShutdownController {
    tx: Mutex<broadcast::Sender<()>>,
    shutdown_triggered: Mutex<bool>,
}

impl ShutdownController {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(1);
        Arc::new(Self {
            tx: Mutex::new(tx),
            shutdown_triggered: Mutex::new(false),
        })
    }

    pub async fn subscribe(&self) -> broadcast::Receiver<()> {
        let tx = self.tx.lock().await;
        tx.subscribe()
    }

    pub async fn trigger_shutdown(&self, reason: &str) {
        let mut triggered = self.shutdown_triggered.lock().await;
        if *triggered {
            debug!(
                log_type = "supervisor",
                "Shutdown already in progress, ignoring additional request"
            );
            return;
        }

        info!(log_type = "supervisor", "Initiating shutdown: {}", reason);
        *triggered = true;

        let tx = self.tx.lock().await;
        let _ = tx.send(());
    }

    pub async fn is_shutdown_triggered(&self) -> bool {
        let triggered = self.shutdown_triggered.lock().await;
        *triggered
    }
}
