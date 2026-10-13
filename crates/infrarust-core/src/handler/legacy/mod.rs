mod login;
mod ping;

use std::sync::Arc;

use bytes::{Buf, BytesMut};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use infrarust_config::UnknownDomainBehavior;
use infrarust_transport::BackendConnector;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::services::ProxyServices;

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
}

impl LegacyHandler {
    pub fn new(
        services: ProxyServices,
        backend_connector: Arc<BackendConnector>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            services,
            backend_connector,
            shutdown,
        }
    }

    fn drops_unknown_domains(&self) -> bool {
        self.services.config.unknown_domain_behavior == UnknownDomainBehavior::Drop
    }

    pub async fn handle(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
        let first_byte = ctx.buffered_data.first().copied().unwrap_or(0);

        match first_byte {
            0xFE => self.handle_ping(ctx).await,
            0x02 => self.handle_login(ctx).await,
            _ => {
                tracing::debug!(byte = first_byte, "unknown legacy first byte");
                Ok(())
            }
        }
    }
}
