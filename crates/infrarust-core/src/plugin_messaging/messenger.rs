use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::messaging::{
    ChannelId, MAX_TO_BACKEND_PAYLOAD, MessagingError, ServerMessenger, private,
};
use infrarust_api::player::Player;
use infrarust_api::types::ServerId;
use infrarust_config::ServerAddress;

use crate::registry::ConnectionRegistry;

pub struct ServerMessengerImpl {
    registry: Arc<ConnectionRegistry>,
}

impl ServerMessengerImpl {
    pub const fn new(registry: Arc<ConnectionRegistry>) -> Self {
        Self { registry }
    }
}

pub(crate) fn send_through_carriers(
    registry: &ConnectionRegistry,
    server: &ServerId,
    channel: &ChannelId,
    data: &Bytes,
) -> usize {
    let mut reached: Vec<ServerAddress> = Vec::new();
    for session in registry.find_by_server(server.as_str()) {
        if !session.is_active() || session.current_server().as_ref() != Some(server) {
            continue;
        }
        let Some(address) = session.connected_address() else {
            continue;
        };
        if reached.contains(&address) {
            continue;
        }
        match session.send_plugin_message_to_backend(channel, data.clone()) {
            Ok(()) => reached.push(address),
            Err(e) => tracing::debug!(
                player = %session.profile().username,
                %server,
                "a player could not carry a plugin message: {e}"
            ),
        }
    }
    reached.len()
}

impl private::Sealed for ServerMessengerImpl {}

impl ServerMessenger for ServerMessengerImpl {
    fn send_to_server(
        &self,
        server: &ServerId,
        channel: &ChannelId,
        data: Bytes,
    ) -> Result<usize, MessagingError> {
        if data.len() > MAX_TO_BACKEND_PAYLOAD {
            return Err(MessagingError::TooLarge {
                size: data.len(),
                max: MAX_TO_BACKEND_PAYLOAD,
            });
        }
        match send_through_carriers(&self.registry, server, channel, &data) {
            0 => Err(MessagingError::NoCarrier),
            sent => Ok(sent),
        }
    }
}
