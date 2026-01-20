use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::net::TcpStream;
use tracing::debug;

const BUFFER_SIZE: usize = 8192;

async fn copy_one_direction(
    input: &TcpStream,
    output: &TcpStream,
    shutdown: &Arc<AtomicBool>,
) -> io::Result<u64> {
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut total_bytes: u64 = 0;
    let mut consecutive_empty_reads = 0u32;

    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(total_bytes);
        }

        let n = match input.try_read(&mut buffer) {
            Ok(0) => {
                // EOF
                return Ok(total_bytes);
            }
            Ok(n) => {
                consecutive_empty_reads = 0; // Reset on progress
                n
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                consecutive_empty_reads += 1;
                if consecutive_empty_reads > 2 {
                    tokio::task::yield_now().await;
                    consecutive_empty_reads = 0;
                }
                input.readable().await?;
                continue;
            }
            Err(e) => return Err(e),
        };

        let mut written = 0;
        while written < n {
            match output.try_write(&buffer[written..n]) {
                Ok(0) => {
                    return Ok(total_bytes);
                }
                Ok(w) => {
                    written += w;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    output.writable().await?;
                }
                Err(e) => return Err(e),
            }
        }
        total_bytes += n as u64;
    }
}

pub async fn copy_bidirectional(
    client: TcpStream,
    server: TcpStream,
    shutdown: Arc<AtomicBool>,
) -> io::Result<(u64, u64)> {
    debug!("Starting userspace fallback transfer");

    // Set TCP_NODELAY for low latency
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;

    let shutdown_c2s = shutdown.clone();
    let shutdown_s2c = shutdown.clone();

    let client_ref = &client;
    let server_ref = &server;

    let result = tokio::select! {
        biased;

        result = copy_one_direction(client_ref, server_ref, &shutdown_c2s) => {
            shutdown.store(true, Ordering::Release);
            let c2s_bytes = result?;
            let s2c_bytes = copy_one_direction(server_ref, client_ref, &shutdown_s2c)
                .await
                .unwrap_or(0);
            Ok((c2s_bytes, s2c_bytes))
        }
        result = copy_one_direction(server_ref, client_ref, &shutdown_s2c) => {
            shutdown.store(true, Ordering::Release);
            let s2c_bytes = result?;
            let c2s_bytes = copy_one_direction(client_ref, server_ref, &shutdown_c2s)
                .await
                .unwrap_or(0);
            Ok((c2s_bytes, s2c_bytes))
        }
    };

    debug!("Userspace fallback transfer completed");
    result
}
