use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tracing::debug;

const SPLICE_MAX: usize = 65536;

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
    mut input: OwnedReadHalf,
    mut output: OwnedWriteHalf,
    pipe: &mut Pipe,
    shutdown: &Arc<AtomicBool>,
) -> io::Result<u64> {
    let mut total_bytes: u64 = 0;

    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(total_bytes);
        }

        let _ = input.read(&mut []).await?;

        loop {
            match pipe.splice_from_fd(input.as_ref().as_raw_fd()) {
                Ok(0) => return Ok(total_bytes), // EOF
                Ok(n) => total_bytes += n as u64,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }

        let _ = output.write(&[]).await?;

        // Output closed
        // Wait for output to be writable again
        loop {
            match pipe.splice_to_fd(output.as_ref().as_raw_fd()) {
                Ok(0) => return Ok(total_bytes), // Pipe empty or output closed
                Ok(_) => {}                      // Continue draining
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
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

    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = server.into_split();
    let mut pipe_c2s = Pipe::new()?;
    let mut pipe_s2c = Pipe::new()?;

    let shutdown_c2s = shutdown.clone();
    let shutdown_s2c = shutdown.clone();

    let result = tokio::select! {
        biased;

        result = splice_one_direction(client_read, server_write, &mut pipe_c2s, &shutdown_c2s) => {
            // Client -> Server completed (client closed or error)
            shutdown.store(true, Ordering::Release);
            let c2s_bytes = result?;
            Ok((c2s_bytes, 0)) // Other direction cancelled
        }
        result = splice_one_direction(server_read, client_write, &mut pipe_s2c, &shutdown_s2c) => {
            // Server -> Client completed (server closed or error)
            shutdown.store(true, Ordering::Release);
            let s2c_bytes = result?;
            Ok((0, s2c_bytes)) // Other direction cancelled
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
