#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
mod fallback;

pub mod types;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tracing::{debug, info};

pub use types::ZeroCopyMessage;

pub fn spawn_splice_task(
    client: TcpStream,
    server: TcpStream,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        info!("Using Linux splice() for zero-copy transfer");
        tokio::spawn(async move {
            match linux::copy_bidirectional(client, server, shutdown).await {
                Ok(result) => result,
                Err(e) => {
                    debug!("Splice transfer error: {}", e);
                    (0, 0)
                }
            }
        })
    }

    #[cfg(not(target_os = "linux"))]
    {
        info!("Using userspace fallback for zero-copy transfer");
        tokio::spawn(async move {
            match fallback::copy_bidirectional(client, server, shutdown).await {
                Ok(result) => result,
                Err(e) => {
                    debug!("Fallback transfer error: {}", e);
                    (0, 0)
                }
            }
        })
    }
}
