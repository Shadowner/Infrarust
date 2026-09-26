use infrarust_api::event::bus::EventBus;
use infrarust_api::events::packet::{RawPacketEvent, RawPacketResult};
use infrarust_api::types::{RawPacket, ServerId};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::{ConnectionState, Direction};

use crate::error::CoreError;
use crate::event_bus::conversion::{protocol_direction_to_api, protocol_state_to_api};
use crate::plugin_messaging::channels::{self, MessageIds};
use crate::plugin_messaging::router;
use crate::session::backend_bridge::BackendBridge;
use crate::session::context::SessionContext;
use crate::session::presentation::PresentationIds;

pub(crate) struct FrameChain<'a> {
    ctx: &'a SessionContext<'a>,
    server: &'a ServerId,
    messages: MessageIds,
    presentation: PresentationIds,
}

impl<'a> FrameChain<'a> {
    pub(crate) fn new(ctx: &'a SessionContext<'a>, server: &'a ServerId) -> Self {
        let registry = ctx.registry();
        let version = ctx.version();
        Self {
            ctx,
            server,
            messages: MessageIds::resolve(registry, version),
            presentation: PresentationIds::resolve(registry, version),
        }
    }

    pub(crate) fn observe(&self, frame: &PacketFrame, state: ConnectionState) {
        let session = &self.ctx.session;
        let bus = &self.ctx.services.event_bus;
        let version = self.ctx.version();
        if self.presentation.client_reply(session, bus, frame, state) {
            return;
        }
        if self.messages.is_information(frame, state) {
            router::observe_information(session, bus, frame, state, version);
        } else if self.messages.is_serverbound(frame, state)
            && let Some(peeked) = channels::peek(frame, version)
        {
            router::observe_client_message(session, bus, &peeked, version);
        }
    }

    pub(crate) async fn serverbound(
        &self,
        frame: PacketFrame,
        state: ConnectionState,
    ) -> Option<PacketFrame> {
        let session = &self.ctx.session;
        let bus = &self.ctx.services.event_bus;
        if self.presentation.client_reply(session, bus, &frame, state) {
            return None;
        }
        if self.messages.is_information(&frame, state) {
            router::observe_information(session, bus, &frame, state, self.ctx.version());
            return Some(frame);
        }
        if self.messages.is_serverbound(&frame, state) {
            return router::from_client(&self.ctx.scope(self.server), frame, state).await;
        }
        Some(frame)
    }

    pub(crate) async fn clientbound(
        &self,
        frame: PacketFrame,
        backend: &mut BackendBridge,
        state: ConnectionState,
    ) -> Result<Option<PacketFrame>, CoreError> {
        let session = &self.ctx.session;
        let bus = &self.ctx.services.event_bus;
        self.presentation.observe_backend(session, &frame, state);
        let frame = if self.presentation.is_transfer(&frame, state) {
            match self
                .presentation
                .transfer_from_backend(session, bus, frame, state)
                .await
            {
                Some(frame) => frame,
                None => return Ok(None),
            }
        } else {
            frame
        };
        if self.messages.is_clientbound(&frame, state) {
            return router::from_backend(&self.ctx.scope(self.server), frame, backend, state).await;
        }
        Ok(Some(frame))
    }

    pub(crate) async fn raw_event(
        &self,
        frame: PacketFrame,
        direction: Direction,
        state: ConnectionState,
    ) -> Option<PacketFrame> {
        let bus = &self.ctx.services.event_bus;
        let api_state = protocol_state_to_api(state);
        let api_direction = protocol_direction_to_api(direction);
        if !bus.has_packet_listeners(frame.id, api_state, api_direction) {
            return Some(frame);
        }
        let mut event = RawPacketEvent::new(
            self.ctx.player_id(),
            api_direction,
            RawPacket::new(frame.id, frame.payload.clone()),
        );
        bus.fire_packet_event(frame.id, api_state, api_direction, &mut event)
            .await;
        match event.result() {
            RawPacketResult::Modify { packet } => {
                Some(PacketFrame::new(packet.packet_id, packet.data.clone()))
            }
            RawPacketResult::Drop => None,
            _ => Some(frame),
        }
    }
}
