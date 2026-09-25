use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::client::{
    PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerSettingsChangedEvent,
};
use infrarust_api::events::messaging::{PluginMessageEvent, PluginMessageResult};
use infrarust_api::events::packet::PacketDirection;
use infrarust_api::messaging::{Endpoint, MessagePhase};
use infrarust_api::player::Player;
use infrarust_api::types::ServerId;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::SConfigClientInformation;
use infrarust_protocol::packets::play::client_information::SClientInformation;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use super::bungeecord;
use super::channels::{
    self, MessageIds, Peeked, is_brand, is_bungeecord, is_register, is_unregister,
    reserved_for_backends,
};
use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::player::PlayerSession;
use crate::player::client_state::to_settings;
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;

pub(crate) const fn phase_of(state: ConnectionState) -> MessagePhase {
    match state {
        ConnectionState::Config => MessagePhase::Configuration,
        _ => MessagePhase::Play,
    }
}

pub(crate) struct ClientObserver {
    session: Arc<PlayerSession>,
    bus: Arc<EventBusImpl>,
    ids: MessageIds,
    version: ProtocolVersion,
}

impl ClientObserver {
    pub(crate) fn new(
        session: &Arc<PlayerSession>,
        services: &ProxyServices,
        version: ProtocolVersion,
    ) -> Self {
        Self {
            session: Arc::clone(session),
            bus: Arc::clone(&services.event_bus),
            ids: MessageIds::resolve(&services.packet_registry, version),
            version,
        }
    }

    pub(crate) fn observe(&self, frame: &PacketFrame, state: ConnectionState) {
        if self.ids.is_information(frame, state) {
            observe_information(&self.session, &self.bus, frame, state, self.version);
        } else if self.ids.is_serverbound(frame, state)
            && let Some(peeked) = channels::peek(frame, self.version)
        {
            observe_client_message(&self.session, &self.bus, &peeked, self.version);
        }
    }
}

fn player(session: &Arc<PlayerSession>) -> Arc<dyn Player> {
    Arc::clone(session) as Arc<dyn Player>
}

pub(crate) fn observe_information(
    session: &Arc<PlayerSession>,
    bus: &EventBusImpl,
    frame: &PacketFrame,
    state: ConnectionState,
    version: ProtocolVersion,
) {
    let mut payload = frame.payload.as_ref();
    let decoded = match state {
        ConnectionState::Config => {
            SConfigClientInformation::decode(&mut payload, version).map(|p| p.information)
        }
        _ => SClientInformation::decode(&mut payload, version).map(|p| p.information),
    };
    let information = match decoded {
        Ok(information) => information,
        Err(e) => {
            tracing::debug!("could not read the client information: {e}");
            return;
        }
    };
    let settings = to_settings(&information);
    if session.client_state().set_information(information) {
        bus.post(PlayerSettingsChangedEvent::new(player(session), settings));
    }
}

fn observe_client_message(
    session: &Arc<PlayerSession>,
    bus: &EventBusImpl,
    peeked: &Peeked,
    version: ProtocolVersion,
) {
    let raw = peeked.channel.as_str();
    if is_brand(raw) {
        if let Some(brand) = channels::parse_brand(&peeked.data, version)
            && session.client_state().set_brand(brand.clone())
        {
            bus.post(PlayerClientBrandEvent::new(player(session), brand));
        }
    } else if is_register(raw) {
        let registered = channels::parse_channels(&peeked.data);
        if !registered.is_empty() {
            session.client_state().add_channels(&registered);
            bus.post(PlayerChannelRegisterEvent::new(
                player(session),
                registered,
                PacketDirection::Serverbound,
            ));
        }
    } else if is_unregister(raw) {
        session
            .client_state()
            .remove_channels(&channels::parse_channels(&peeked.data));
    }
}

fn observe_backend_message(session: &Arc<PlayerSession>, bus: &EventBusImpl, peeked: &Peeked) {
    if is_register(&peeked.channel) {
        let registered = channels::parse_channels(&peeked.data);
        if !registered.is_empty() {
            bus.post(PlayerChannelRegisterEvent::new(
                player(session),
                registered,
                PacketDirection::Clientbound,
            ));
        }
    }
}

pub(crate) struct Scope<'a> {
    pub(crate) services: &'a ProxyServices,
    pub(crate) session: &'a Arc<PlayerSession>,
    pub(crate) server: &'a ServerId,
    pub(crate) version: ProtocolVersion,
}

pub(crate) async fn from_client(
    scope: &Scope<'_>,
    frame: PacketFrame,
    state: ConnectionState,
) -> Option<PacketFrame> {
    let Some(peeked) = channels::peek(&frame, scope.version) else {
        return Some(frame);
    };
    observe_client_message(
        scope.session,
        &scope.services.event_bus,
        &peeked,
        scope.version,
    );
    if reserved_for_backends(&peeked.channel) {
        tracing::debug!(
            player = %scope.session.profile().username,
            channel = %peeked.channel,
            "dropping a client plugin message on a channel reserved for backends"
        );
        return None;
    }
    plugin_event(scope, Endpoint::Client, frame, peeked, state)
        .await
        .map(|(frame, _)| frame)
}

pub(crate) async fn from_backend(
    scope: &Scope<'_>,
    frame: PacketFrame,
    backend: &mut BackendBridge,
    state: ConnectionState,
) -> Result<Option<PacketFrame>, CoreError> {
    let Some(peeked) = channels::peek(&frame, scope.version) else {
        return Ok(Some(frame));
    };
    observe_backend_message(scope.session, &scope.services.event_bus, &peeked);
    let bungee = is_bungeecord(&peeked.channel);
    let source = Endpoint::Backend(scope.server.clone());
    let Some((frame, data)) = plugin_event(scope, source, frame, peeked, state).await else {
        return Ok(None);
    };
    if bungee
        && scope.services.plugin_messaging.bungeecord_enabled()
        && let Some(config) = scope
            .services
            .domain_router
            .find_by_server_id(scope.server.as_str())
        && scope.services.plugin_messaging.bungeecord_for(&config)
    {
        bungeecord::handle(scope, &config, &data, backend, state).await?;
        return Ok(None);
    }
    Ok(Some(frame))
}

async fn plugin_event(
    scope: &Scope<'_>,
    source: Endpoint,
    frame: PacketFrame,
    peeked: Peeked,
    state: ConnectionState,
) -> Option<(PacketFrame, Bytes)> {
    let Some(channel) = scope
        .services
        .plugin_messaging
        .channels()
        .lookup(&peeked.channel)
    else {
        return Some((frame, peeked.data));
    };
    let Peeked { channel: raw, data } = peeked;
    let event = PluginMessageEvent::new(
        player(scope.session),
        source,
        channel,
        raw.clone(),
        data.clone(),
        phase_of(state),
    );
    let event = scope.services.event_bus.fire(event).await;
    match event.result() {
        PluginMessageResult::Handled => None,
        PluginMessageResult::Replace(replaced) => Some((
            channels::build(frame.id, &raw, replaced, scope.version),
            replaced.clone(),
        )),
        _ => Some((frame, data)),
    }
}
