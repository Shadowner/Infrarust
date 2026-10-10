use std::sync::Arc;

use infrarust_api::events::connection::{
    ConnectCause, ServerConnectedEvent, ServerPostConnectEvent, ServerPreConnectEvent,
};
use infrarust_api::player::Player;
use infrarust_api::types::ServerId;

use crate::event_bus::EventBusImpl;
use crate::player::PlayerSession;

pub(crate) async fn pre_connect(
    bus: &EventBusImpl,
    player: &Arc<PlayerSession>,
    server: ServerId,
    cause: ConnectCause,
) -> ServerPreConnectEvent {
    bus.fire(ServerPreConnectEvent::new(
        Arc::clone(player) as Arc<dyn Player>,
        server,
        player.current_server(),
        cause,
    ))
    .await
}

pub(crate) struct ServerJoin {
    player: Arc<PlayerSession>,
    server: ServerId,
    previous_server: Option<ServerId>,
    connected: bool,
}

impl ServerJoin {
    pub(crate) fn new(player: &Arc<PlayerSession>, server: ServerId) -> Self {
        Self {
            previous_server: player.current_server(),
            player: Arc::clone(player),
            server,
            connected: false,
        }
    }

    pub(crate) async fn connected(&mut self, bus: &EventBusImpl) {
        if self.connected {
            return;
        }
        self.connected = true;
        bus.fire(ServerConnectedEvent::new(
            Arc::clone(&self.player) as Arc<dyn Player>,
            self.server.clone(),
            self.previous_server.clone(),
        ))
        .await;
    }

    pub(crate) async fn joined(mut self, bus: &EventBusImpl) -> ServerId {
        self.connected(bus).await;
        self.player.set_current_server(self.server.clone());
        bus.fire(ServerPostConnectEvent::new(
            Arc::clone(&self.player) as Arc<dyn Player>,
            self.server.clone(),
            self.previous_server,
        ))
        .await;
        self.server
    }
}
