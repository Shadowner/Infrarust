use std::sync::Arc;

use bytes::Bytes;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::resource_pack::{PlayerResourcePackStatusEvent, ResourcePackOrigin};
use infrarust_api::events::transfer::{PreTransferEvent, PreTransferResult, TransferOrigin};
use infrarust_api::player::{Player, ResourcePackStatus, cookie_key};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::cookie::{
    CConfigCookieRequest, CConfigStoreCookie, CCookieRequest, SConfigCookieResponse,
    SCookieResponse,
};
use infrarust_protocol::packets::play::boss_bar::{BossBarAction, CBossBar};
use infrarust_protocol::packets::play::transfer::{CConfigTransfer, CTransfer};
use infrarust_protocol::packets::resource_pack::{
    CConfigResourcePack, CConfigResourcePackPop, CConfigResourcePackPush, CResourcePack,
    ResourcePackResult, SConfigResourcePackResponse, SResourcePackResponse,
};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::player::PlayerSession;
use crate::player::packets::{
    boss_bar_added, boss_bar_removed, build_header_footer, encode_packet,
};
use crate::session::client_bridge::ClientBridge;

#[derive(Debug, Clone, Copy, Default)]
struct Ids {
    cookie_response: Option<i32>,
    pack_response: Option<i32>,
    cookie_request: Option<i32>,
    legacy_pack: Option<i32>,
    transfer: Option<i32>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PresentationIds {
    version: ProtocolVersion,
    play: Ids,
    config: Ids,
    backend_boss_bar: Option<i32>,
    config_player_only: [Option<i32>; 6],
}

fn legacy_packs(version: ProtocolVersion) -> bool {
    version.less_than(ProtocolVersion::V1_20_3)
}

impl PresentationIds {
    pub(crate) fn resolve(registry: &PacketRegistry, version: ProtocolVersion) -> Self {
        let legacy = legacy_packs(version);
        Self {
            version,
            play: Ids {
                cookie_response: registry.get_packet_id::<SCookieResponse>(version),
                pack_response: registry.get_packet_id::<SResourcePackResponse>(version),
                cookie_request: registry.get_packet_id::<CCookieRequest>(version),
                legacy_pack: registry
                    .get_packet_id::<CResourcePack>(version)
                    .filter(|_| legacy),
                transfer: registry.get_packet_id::<CTransfer>(version),
            },
            config: Ids {
                cookie_response: registry.get_packet_id::<SConfigCookieResponse>(version),
                pack_response: registry.get_packet_id::<SConfigResourcePackResponse>(version),
                cookie_request: registry.get_packet_id::<CConfigCookieRequest>(version),
                legacy_pack: registry
                    .get_packet_id::<CConfigResourcePack>(version)
                    .filter(|_| legacy),
                transfer: registry.get_packet_id::<CConfigTransfer>(version),
            },
            backend_boss_bar: registry
                .get_packet_id::<CBossBar>(version)
                .filter(|_| version.less_than(ProtocolVersion::V1_20_2)),
            config_player_only: [
                registry.get_packet_id::<CConfigCookieRequest>(version),
                registry.get_packet_id::<CConfigStoreCookie>(version),
                registry.get_packet_id::<CConfigResourcePack>(version),
                registry.get_packet_id::<CConfigResourcePackPush>(version),
                registry.get_packet_id::<CConfigResourcePackPop>(version),
                registry.get_packet_id::<CConfigTransfer>(version),
            ],
        }
    }

    pub(crate) fn is_player_request(&self, frame: &PacketFrame, state: ConnectionState) -> bool {
        state == ConnectionState::Config && self.config_player_only.contains(&Some(frame.id))
    }

    const fn ids(&self, state: ConnectionState) -> Option<&Ids> {
        match state {
            ConnectionState::Play => Some(&self.play),
            ConnectionState::Config => Some(&self.config),
            _ => None,
        }
    }

    pub(crate) fn client_reply(
        &self,
        session: &Arc<PlayerSession>,
        bus: &EventBusImpl,
        frame: &PacketFrame,
        state: ConnectionState,
    ) -> bool {
        let Some(ids) = self.ids(state) else {
            return false;
        };
        let id = Some(frame.id);
        if id == ids.cookie_response {
            return self.cookie_answered(session, frame, state);
        }
        if id == ids.pack_response {
            return self.pack_answered(session, bus, frame, state);
        }
        false
    }

    fn cookie_answered(
        &self,
        session: &PlayerSession,
        frame: &PacketFrame,
        state: ConnectionState,
    ) -> bool {
        let mut payload = frame.payload.as_ref();
        let decoded = match state {
            ConnectionState::Config => SConfigCookieResponse::decode(&mut payload, self.version)
                .map(|p| (p.key, p.payload)),
            _ => SCookieResponse::decode(&mut payload, self.version).map(|p| (p.key, p.payload)),
        };
        match decoded {
            Ok((key, payload)) => session.presentation().cookie_answered(&key, payload),
            Err(e) => {
                tracing::debug!("could not read a cookie response: {e}");
                false
            }
        }
    }

    fn pack_answered(
        &self,
        session: &Arc<PlayerSession>,
        bus: &EventBusImpl,
        frame: &PacketFrame,
        state: ConnectionState,
    ) -> bool {
        let mut payload = frame.payload.as_ref();
        let decoded = match state {
            ConnectionState::Config => {
                SConfigResourcePackResponse::decode(&mut payload, self.version)
                    .map(|p| (p.id, p.result))
            }
            _ => {
                SResourcePackResponse::decode(&mut payload, self.version).map(|p| (p.id, p.result))
            }
        };
        let (id, result): (_, ResourcePackResult) = match decoded {
            Ok(decoded) => decoded,
            Err(e) => {
                tracing::debug!("could not read a resource pack response: {e}");
                return false;
            }
        };
        let status = ResourcePackStatus::from_id(result.id());
        let (origin, pack_id) =
            session
                .presentation()
                .pack_answered(id, status, legacy_packs(self.version));
        bus.post(PlayerResourcePackStatusEvent::new(
            Arc::clone(session) as Arc<dyn Player>,
            pack_id,
            status,
            origin,
        ));
        origin == ResourcePackOrigin::Proxy
    }

    pub(crate) fn observe_backend(
        &self,
        session: &PlayerSession,
        frame: &PacketFrame,
        state: ConnectionState,
    ) {
        let Some(ids) = self.ids(state) else {
            return;
        };
        let id = Some(frame.id);
        if id == ids.cookie_request {
            self.backend_cookie_request(session, frame, state);
        } else if id == ids.legacy_pack {
            session.presentation().pack_pushed(None, false, true);
        } else if state == ConnectionState::Play && id == self.backend_boss_bar {
            self.backend_boss_bar(session, frame);
        }
    }

    fn backend_cookie_request(
        &self,
        session: &PlayerSession,
        frame: &PacketFrame,
        state: ConnectionState,
    ) {
        let mut payload = frame.payload.as_ref();
        let key = match state {
            ConnectionState::Config => {
                CConfigCookieRequest::decode(&mut payload, self.version).map(|p| p.key)
            }
            _ => CCookieRequest::decode(&mut payload, self.version).map(|p| p.key),
        };
        match key {
            Ok(key) => {
                let key = cookie_key(&key).unwrap_or(key);
                session.presentation().cookie_requested(&key, None);
            }
            Err(e) => tracing::debug!("could not read a backend cookie request: {e}"),
        }
    }

    fn backend_boss_bar(&self, session: &PlayerSession, frame: &PacketFrame) {
        match CBossBar::decode(&mut frame.payload.as_ref(), self.version) {
            Ok(CBossBar {
                id,
                action: BossBarAction::Add { .. },
            }) => session.presentation().backend_bar(id, true),
            Ok(CBossBar {
                id,
                action: BossBarAction::Remove,
            }) => session.presentation().backend_bar(id, false),
            Ok(_) => {}
            Err(e) => tracing::debug!("could not read a backend boss bar: {e}"),
        }
    }

    pub(crate) fn is_transfer(&self, frame: &PacketFrame, state: ConnectionState) -> bool {
        self.ids(state)
            .is_some_and(|ids| ids.transfer == Some(frame.id))
    }

    pub(crate) async fn transfer_from_backend(
        &self,
        session: &Arc<PlayerSession>,
        bus: &EventBusImpl,
        frame: PacketFrame,
        state: ConnectionState,
    ) -> Option<PacketFrame> {
        let mut payload = frame.payload.as_ref();
        let decoded = match state {
            ConnectionState::Config => {
                CConfigTransfer::decode(&mut payload, self.version).map(|p| (p.host, p.port))
            }
            _ => CTransfer::decode(&mut payload, self.version).map(|p| (p.host, p.port)),
        };
        let Some((host, port)) = decoded
            .ok()
            .and_then(|(host, port)| Some((host, u16::try_from(port).ok()?)))
        else {
            return Some(frame);
        };
        let event = bus
            .fire(PreTransferEvent::new(
                Arc::clone(session) as Arc<dyn Player>,
                host,
                port,
                TransferOrigin::Backend,
            ))
            .await;
        match event.result() {
            PreTransferResult::Denied { .. } => {
                tracing::info!(
                    player = %session.profile().username,
                    host = %event.host,
                    port = event.port,
                    "a backend transfer was denied by a plugin"
                );
                None
            }
            PreTransferResult::Redirect { host, port } => {
                let port = i32::from(*port);
                let mut payload = Vec::new();
                let encoded = match state {
                    ConnectionState::Config => CConfigTransfer {
                        host: host.clone(),
                        port,
                    }
                    .encode(&mut payload, self.version),
                    _ => CTransfer {
                        host: host.clone(),
                        port,
                    }
                    .encode(&mut payload, self.version),
                };
                match encoded {
                    Ok(()) => Some(PacketFrame::new(frame.id, Bytes::from(payload))),
                    Err(e) => {
                        tracing::warn!("could not redirect a backend transfer: {e}");
                        Some(frame)
                    }
                }
            }
            _ => Some(frame),
        }
    }
}

pub(crate) async fn restore_after_switch(
    client: &mut ClientBridge,
    session: &PlayerSession,
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> Result<(), CoreError> {
    let presentation = session.presentation();
    let stale = presentation.take_backend_bars();
    let mut frames = Vec::new();
    if version.no_less_than(ProtocolVersion::V1_9) {
        if version.less_than(ProtocolVersion::V1_20_2) {
            for id in stale {
                frames.push(encode_packet(&boss_bar_removed(id), version, registry)?);
            }
        } else {
            for (id, bar) in presentation.bars() {
                frames.push(encode_packet(
                    &boss_bar_added(id, &bar, version),
                    version,
                    registry,
                )?);
            }
        }
    }
    if version.no_less_than(ProtocolVersion::V1_8)
        && let Some((header, footer)) = presentation.header_footer()
    {
        frames.push(build_header_footer(&header, &footer, version, registry)?);
    }
    if frames.is_empty() {
        return Ok(());
    }
    for frame in &frames {
        client.queue_frame(frame)?;
    }
    client.flush().await
}
