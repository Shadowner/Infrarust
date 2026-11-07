//! Socket writes bounded in time, poisoned for good once they fail or are dropped.

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::error::CoreError;

const WRITE_LIMIT: Duration = Duration::from_secs(30);
const CANCEL_GRACE: Duration = Duration::from_secs(5);

pub(crate) struct WriteGuard {
    broken: bool,
    cancel: CancellationToken,
}

impl WriteGuard {
    pub(crate) fn new() -> Self {
        Self {
            broken: false,
            cancel: CancellationToken::new(),
        }
    }

    pub(crate) fn set_cancel(&mut self, cancel: CancellationToken) {
        self.cancel = cancel;
    }

    pub(crate) const fn check(&self) -> Result<(), CoreError> {
        if self.broken {
            return Err(CoreError::ConnectionClosed);
        }
        Ok(())
    }

    pub(crate) async fn write_all(
        &mut self,
        stream: &mut TcpStream,
        data: &[u8],
    ) -> Result<(), CoreError> {
        self.check()?;
        self.broken = true;
        write_bounded(stream, data, &self.cancel).await?;
        self.broken = false;
        Ok(())
    }
}

async fn write_bounded(
    stream: &mut TcpStream,
    mut data: &[u8],
    cancel: &CancellationToken,
) -> Result<(), CoreError> {
    while !data.is_empty() {
        match stream.try_write(data) {
            Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into()),
            Ok(written) => data = &data[written..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                return write_stalled(stream, data, cancel).await;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

async fn write_stalled(
    stream: &mut TcpStream,
    data: &[u8],
    cancel: &CancellationToken,
) -> Result<(), CoreError> {
    let write = stream.write_all(data);
    tokio::pin!(write);
    tokio::select! {
        written = &mut write => Ok(written?),
        () = tokio::time::sleep(WRITE_LIMIT) => Err(stalled()),
        () = cancel.cancelled() => match tokio::time::timeout(CANCEL_GRACE, write).await {
            Ok(written) => Ok(written?),
            Err(_) => Err(stalled()),
        },
    }
}

fn stalled() -> CoreError {
    CoreError::Timeout("the peer stopped reading".to_string())
}
