use std::collections::HashMap;
use std::sync::Arc;

use bytes::{BufMut, Bytes, BytesMut};

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::messaging::{
    LoginPluginRequestEvent, LoginPluginRequestResult, LoginPluginResponseEvent,
    LoginPluginResponseResult,
};
use infrarust_api::player::Player;
use infrarust_api::types::ServerId;
use infrarust_protocol::VarInt;
use infrarust_protocol::codec::McBufReadExt;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::login::{CLoginPluginRequest, SLoginPluginResponse};
use infrarust_protocol::version::ProtocolVersion;

use crate::forwarding::velocity::is_velocity_request;
use crate::player::PlayerSession;
use crate::services::ProxyServices;
use crate::session::context::SessionContext;

pub(crate) enum Routed {
    Client(Option<PacketFrame>),
    Backend(SLoginPluginResponse),
}

struct Watch {
    requests: bool,
    responses: bool,
}

pub(crate) struct LoginMessages<'a> {
    services: &'a ProxyServices,
    session: &'a Arc<PlayerSession>,
    server: ServerId,
    version: ProtocolVersion,
    relayed: HashMap<i32, String>,
}

impl<'a> LoginMessages<'a> {
    pub(crate) fn new(ctx: &'a SessionContext<'_>, server: ServerId) -> Self {
        Self {
            services: ctx.services,
            session: &ctx.session,
            server,
            version: ctx.version(),
            relayed: HashMap::new(),
        }
    }

    pub(crate) async fn relay(
        &mut self,
        frame: &PacketFrame,
        request: &CLoginPluginRequest,
    ) -> Routed {
        let Some(watch) = self.watch(request) else {
            return Routed::Client(None);
        };
        let result = if watch.requests {
            self.decide(frame, request, true).await
        } else {
            LoginPluginRequestResult::Forward
        };
        let replaced = match result {
            LoginPluginRequestResult::Respond(data) => {
                return Routed::Backend(answered(request, data));
            }
            LoginPluginRequestResult::Replace(data) => {
                tracing::debug!(
                    channel = %request.channel,
                    message_id = request.message_id.0,
                    "a plugin replaced a login plugin request"
                );
                Some(with_data(frame, &request.data, &data))
            }
            _ => None,
        };
        if watch.responses {
            self.relayed
                .insert(request.message_id.0, request.channel.clone());
        }
        Routed::Client(replaced)
    }

    pub(crate) async fn answer(
        &self,
        frame: &PacketFrame,
        request: &CLoginPluginRequest,
    ) -> SLoginPluginResponse {
        if self.watch(request).is_some_and(|watch| watch.requests)
            && let LoginPluginRequestResult::Respond(data) =
                self.decide(frame, request, false).await
        {
            return answered(request, data);
        }
        tracing::debug!(
            channel = %request.channel,
            "responding 'not understood' to LoginPluginRequest"
        );
        response_to(request.message_id, None)
    }

    pub(crate) async fn response(&mut self, frame: &PacketFrame) -> Option<PacketFrame> {
        if self.relayed.is_empty() {
            return None;
        }
        let message_id = frame.payload.as_ref().read_var_int().ok()?.0;
        let channel = self.relayed.remove(&message_id)?;
        let response = match SLoginPluginResponse::decode(&mut frame.payload.as_ref(), self.version)
        {
            Ok(response) => response,
            Err(e) => {
                tracing::debug!(%channel, message_id, "could not read a login plugin response: {e}");
                return None;
            }
        };
        let start = data_start(frame, &response.data);
        let event = LoginPluginResponseEvent::new(
            self.player(),
            self.server.clone(),
            message_id,
            channel,
            response.successful,
            frame.payload.slice(start..),
        );
        let event = self.services.event_bus.fire(event).await;
        let LoginPluginResponseResult::Replace { successful, data } = event.result() else {
            return None;
        };
        tracing::debug!(
            channel = %event.channel,
            message_id,
            "a plugin replaced a login plugin response"
        );
        let flag = start.saturating_sub(1);
        let mut payload = BytesMut::with_capacity(flag + 1 + data.len());
        payload.extend_from_slice(&frame.payload[..flag]);
        payload.put_u8(u8::from(*successful));
        payload.extend_from_slice(data);
        Some(PacketFrame::new(frame.id, payload.freeze()))
    }

    fn watch(&self, request: &CLoginPluginRequest) -> Option<Watch> {
        if is_velocity_request(request) {
            return None;
        }
        self.services
            .plugin_messaging
            .channels()
            .lookup(&request.channel)?;
        let bus = &self.services.event_bus;
        let watch = Watch {
            requests: bus.has_listeners::<LoginPluginRequestEvent>(),
            responses: bus.has_listeners::<LoginPluginResponseEvent>(),
        };
        (watch.requests || watch.responses).then_some(watch)
    }

    async fn decide(
        &self,
        frame: &PacketFrame,
        request: &CLoginPluginRequest,
        client_reachable: bool,
    ) -> LoginPluginRequestResult {
        let event = LoginPluginRequestEvent::new(
            self.player(),
            self.server.clone(),
            request.message_id.0,
            request.channel.clone(),
            frame.payload.slice(data_start(frame, &request.data)..),
            client_reachable,
        );
        self.services.event_bus.fire(event).await.result().clone()
    }

    fn player(&self) -> Arc<dyn Player> {
        Arc::clone(self.session) as Arc<dyn Player>
    }
}

fn answered(request: &CLoginPluginRequest, data: Option<Bytes>) -> SLoginPluginResponse {
    tracing::debug!(
        channel = %request.channel,
        message_id = request.message_id.0,
        understood = data.is_some(),
        "a plugin answered a login plugin request"
    );
    response_to(request.message_id, data)
}

fn response_to(message_id: VarInt, data: Option<Bytes>) -> SLoginPluginResponse {
    SLoginPluginResponse {
        message_id,
        successful: data.is_some(),
        data: data.map(Vec::from).unwrap_or_default(),
    }
}

fn data_start(frame: &PacketFrame, data: &[u8]) -> usize {
    frame.payload.len().saturating_sub(data.len())
}

fn with_data(frame: &PacketFrame, old: &[u8], data: &[u8]) -> PacketFrame {
    let head = data_start(frame, old);
    let mut payload = BytesMut::with_capacity(head + data.len());
    payload.extend_from_slice(&frame.payload[..head]);
    payload.extend_from_slice(data);
    PacketFrame::new(frame.id, payload.freeze())
}

