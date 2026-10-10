use infrarust_api::error::PlayerError;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::transfer::{PreTransferEvent, PreTransferResult, TransferOrigin};

use super::PlayerSession;

impl PlayerSession {
    pub(super) async fn approve_transfer(
        &self,
        host: String,
        port: u16,
    ) -> Result<(String, u16), PlayerError> {
        let (Some(bus), Some(player)) = (&self.events, self.shared_player()) else {
            return Ok((host, port));
        };
        let event = bus
            .fire(PreTransferEvent::new(
                player,
                host,
                port,
                TransferOrigin::Plugin,
            ))
            .await;
        match event.result() {
            PreTransferResult::Denied { reason } => {
                Err(PlayerError::Denied(Box::new(reason.clone())))
            }
            PreTransferResult::Redirect { host, port } => Ok((host.clone(), *port)),
            _ => Ok((event.host, event.port)),
        }
    }
}
