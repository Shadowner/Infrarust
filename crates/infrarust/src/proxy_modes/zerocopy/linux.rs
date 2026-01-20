use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::Interest;
use tokio::net::TcpStream;
use tracing::debug;

const PIPE_CAPACITY: usize = 65536;

const SPLICE_MAX: usize = PIPE_CAPACITY;

const SPLICE_FLAGS: libc::c_uint = libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK;

pub struct Pipe {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
}

impl Pipe {
    pub fn new() -> io::Result<Self> {
        let mut fds = [0i32; 2];
        let result = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            read_fd: unsafe { OwnedFd::from_raw_fd(fds[0]) },
            write_fd: unsafe { OwnedFd::from_raw_fd(fds[1]) },
        })
    }

    fn splice_from_fd(&self, fd: i32) -> io::Result<usize> {
        let result = unsafe {
            libc::splice(
                fd,
                std::ptr::null_mut(),
                self.write_fd.as_raw_fd(),
                std::ptr::null_mut(),
                SPLICE_MAX,
                SPLICE_FLAGS,
            )
        };

        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result as usize)
        }
    }

    fn splice_to_fd(&self, fd: i32) -> io::Result<usize> {
        let result = unsafe {
            libc::splice(
                self.read_fd.as_raw_fd(),
                std::ptr::null_mut(),
                fd,
                std::ptr::null_mut(),
                SPLICE_MAX,
                SPLICE_FLAGS,
            )
        };

        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result as usize)
        }
    }
}

async fn splice_one_direction(
    input: &TcpStream,
    output: &TcpStream,
    pipe: &Pipe,
    shutdown: &Arc<AtomicBool>,
) -> io::Result<u64> {
    let input_fd = input.as_raw_fd();
    let output_fd = output.as_raw_fd();
    let mut total_bytes: u64 = 0;
    let mut consecutive_empty_reads = 0u32;

    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(total_bytes);
        }

        let mut bytes_in_pipe = 0usize;
        loop {
            match pipe.splice_from_fd(input_fd) {
                Ok(0) => {
                    // EOF from input
                    return Ok(total_bytes);
                }
                Ok(n) => {
                    bytes_in_pipe += n;
                    total_bytes += n as u64;
                    consecutive_empty_reads = 0; // Reset on progress
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        if bytes_in_pipe > 0 {
            while bytes_in_pipe > 0 {
                match pipe.splice_to_fd(output_fd) {
                    Ok(0) => {
                        return Ok(total_bytes);
                    }
                    Ok(n) => {
                        bytes_in_pipe -= n;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        output.ready(Interest::WRITABLE).await?;
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
            continue;
        }

        consecutive_empty_reads += 1;

                    // Output closed
                    // Wait for output to be writable again
        if consecutive_empty_reads > 2 {
            tokio::task::yield_now().await;
            consecutive_empty_reads = 0;
        }

        input.ready(Interest::READABLE).await?;
    }
}

pub async fn copy_bidirectional(
    client: TcpStream,
    server: TcpStream,
    shutdown: Arc<AtomicBool>,
) -> io::Result<(u64, u64)> {
    debug!("Starting zero-copy splice transfer");

    client.set_nodelay(true)?;
    server.set_nodelay(true)?;

    let pipe_c2s = Pipe::new()?;
    let pipe_s2c = Pipe::new()?;

    let shutdown_c2s = shutdown.clone();
    let shutdown_s2c = shutdown.clone();

    let client_ref = &client;
    let server_ref = &server;

    let result = tokio::select! {
        biased;

        result = splice_one_direction(client_ref, server_ref, &pipe_c2s, &shutdown_c2s) => {
            // Client -> Server completed (client closed or error)
            shutdown.store(true, Ordering::Release);
            let c2s_bytes = result?;
            // Drain remaining server -> client
            let s2c_bytes = splice_one_direction(server_ref, client_ref, &pipe_s2c, &shutdown_s2c)
                .await
                .unwrap_or(0);
            Ok((c2s_bytes, s2c_bytes))
        }
        result = splice_one_direction(server_ref, client_ref, &pipe_s2c, &shutdown_s2c) => {
            // Server -> Client completed (server closed or error)
            shutdown.store(true, Ordering::Release);
            let s2c_bytes = result?;
            // Drain remaining client -> server
            let c2s_bytes = splice_one_direction(client_ref, server_ref, &pipe_c2s, &shutdown_c2s)
                .await
                .unwrap_or(0);
            Ok((c2s_bytes, s2c_bytes))
        }
    };

    debug!("Splice transfer completed");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipe_creation() {
        let pipe = Pipe::new();
        assert!(pipe.is_ok());
    }
}
