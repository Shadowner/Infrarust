pub(crate) mod hello;
mod login;
mod ping;

use std::sync::Arc;

use bytes::{Buf, BytesMut};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use crate::pipeline::context::ConnectionContext;
use crate::services::ProxyServices;
use crate::status::StatusCache;
use crate::status::relay_gate::RelayGate;

struct LegacyReader<'a> {
    ctx: &'a mut ConnectionContext,
    read_ahead: BytesMut,
}

impl<'a> LegacyReader<'a> {
    fn new(ctx: &'a mut ConnectionContext) -> Self {
        let first = ctx.buffered_data.len().min(1);
        let read_ahead = ctx.buffered_data.split_off(first);
        Self { ctx, read_ahead }
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        let buffered = buf.len().min(self.read_ahead.len());
        self.read_ahead.copy_to_slice(&mut buf[..buffered]);
        if buffered < buf.len() {
            self.ctx
                .stream_mut()
                .read_exact(&mut buf[buffered..])
                .await?;
        }
        Ok(())
    }

    async fn read_u8(&mut self) -> std::io::Result<u8> {
        let mut byte = [0u8; 1];
        self.read_exact(&mut byte).await?;
        Ok(byte[0])
    }

    async fn read_vec(&mut self, len: usize) -> std::io::Result<Vec<u8>> {
        let mut bytes = vec![0u8; len];
        self.read_exact(&mut bytes).await?;
        Ok(bytes)
    }

    fn unread(self) -> BytesMut {
        self.read_ahead
    }
}

pub struct LegacyHandler {
    services: ProxyServices,
    backend_connector: Arc<BackendConnector>,
    shutdown: CancellationToken,
    status_cache: Arc<StatusCache>,
    relay_gate: Arc<RelayGate>,
}

impl LegacyHandler {
    pub fn new(
        services: ProxyServices,
        backend_connector: Arc<BackendConnector>,
        shutdown: CancellationToken,
        status_cache: Arc<StatusCache>,
        relay_gate: Arc<RelayGate>,
    ) -> Self {
        Self {
            services,
            backend_connector,
            shutdown,
            status_cache,
            relay_gate,
        }
    }
}
