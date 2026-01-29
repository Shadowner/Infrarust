#[cfg(target_os = "linux")]
mod linux;

pub mod types;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::net::TcpStream;
use tokio::task::JoinHandle;
#[cfg(not(target_os = "linux"))]
use tracing::error;
#[cfg(target_os = "linux")]
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
        let _ = (client, server);

        tokio::spawn(async move {
            error!(
                "ZeroCopy mode is not available: requires Linux with splice() syscall support. \
                 This platform does not support zero-copy networking."
            );
            shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
            (0, 0)
        })
    }
}
