use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::claim::{LoginClaimEvent, LoginClaimResult};
use infrarust_api::types::{ProtocolVersion, ServerId};

use crate::ban::BanManager;
use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::pipeline::admission;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{
    ClaimedLogin, HandshakeData, LayoutRefusal, LoginData, LoginStartLayout, Refused, RoutingData,
};

pub struct LoginClaimMiddleware {
    bus: Arc<EventBusImpl>,
    bans: Arc<BanManager>,
}

impl LoginClaimMiddleware {
    pub const fn new(bus: Arc<EventBusImpl>, bans: Arc<BanManager>) -> Self {
        Self { bus, bans }
    }
}

impl Middleware for LoginClaimMiddleware {
    fn name(&self) -> &'static str {
        "login_claim"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let refusal = ctx.extensions.remove::<LayoutRefusal>();
            if self.bus.has_listeners::<LoginClaimEvent>()
                && let Some(event) = claim_event(ctx)
            {
                let event = self.bus.fire(event).await;
                if let LoginClaimResult::Handoff(handoff) = event.result() {
                    if let Some(refusal) = admission::refuse_handoff(&self.bans, ctx).await {
                        ctx.extensions.insert(Refused(refusal.reason));
                        return Ok(MiddlewareResult::Kick(refusal.message));
                    }
                    tracing::debug!(
                        client = %ctx.client_addr(),
                        username = %event.username,
                        target = %handoff.target,
                        layout_matches = event.layout_matches,
                        "login claimed by a plugin"
                    );
                    ctx.extensions.insert(ClaimedLogin(handoff.clone()));
                    return Ok(MiddlewareResult::ShortCircuit);
                }
            }
            match refusal {
                Some(LayoutRefusal(error)) => Err(error.into()),
                None => Ok(MiddlewareResult::Continue),
            }
        })
    }
}

fn claim_event(ctx: &ConnectionContext) -> Option<LoginClaimEvent> {
    let handshake = ctx.extensions.get::<HandshakeData>()?;
    let login = ctx.extensions.get::<LoginData>()?;
    let server = ctx
        .extensions
        .get::<RoutingData>()
        .map(|routing| ServerId::new(routing.config_id.clone()));
    Some(
        LoginClaimEvent::new(
            ctx.client_addr(),
            ProtocolVersion::new(handshake.protocol_version.0),
            login.username.clone(),
            login.raw_payload.clone(),
        )
        .with_host(
            handshake.raw_host.clone(),
            Some(handshake.domain.clone()),
            handshake.port,
        )
        .with_server(server)
        .with_uuid(login.player_uuid)
        .with_layout_matches(login.layout == LoginStartLayout::Declared),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bytes::{Bytes, BytesMut};
    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::events::handshake::RejectReason;
    use infrarust_api::handoff::{Handoff, HandoffTarget};
    use infrarust_api::services::ban_service::{BanRequest, BanTarget};
    use infrarust_config::{ProxyMode, ServerConfig};
    use infrarust_protocol::ProtocolError;
    use tokio::net::{TcpListener, TcpStream};
    use uuid::Uuid;

    use super::*;
    use crate::ban::{BAN_CHECK_UNAVAILABLE, FileBanStorage};
    use crate::pipeline::types::{ConnectionIntent, Refused};
    use crate::session::connection_registry::ConnectionRegistry;

    const UUID: Uuid = Uuid::from_u128(0xa7c3_5e10_9b2d_4f6e_8a01_c4d2_e3f4_a5b6);
    const PAYLOAD: &[u8] = b"\x05probe0123456789abcdef";

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Seen {
        remote_addr: SocketAddr,
        virtual_host: Option<String>,
        raw_host: String,
        port: u16,
        protocol: i32,
        server: Option<ServerId>,
        username: String,
        uuid: Option<Uuid>,
        login_start: Bytes,
        layout_matches: bool,
    }

    async fn claim_ctx(layout: LoginStartLayout, refused: bool) -> (ConnectionContext, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, peer) = listener.accept().await.unwrap();
        let mut ctx =
            ConnectionContext::new_for_test(server, peer, IpAddr::V4(Ipv4Addr::LOCALHOST), addr);
        ctx.extensions.insert(HandshakeData {
            domain: "lobby.test".to_string(),
            raw_host: "Lobby.Test\0FML3\0".to_string(),
            port: 25577,
            protocol_version: infrarust_protocol::ProtocolVersion(763),
            intent: ConnectionIntent::Login,
            raw_packets: vec![BytesMut::from(&b"handshake"[..])],
        });
        let mut config: ServerConfig =
            toml::from_str("domains = [\"lobby.test\"]\naddresses = [\"127.0.0.1:25565\"]\n")
                .unwrap();
        config.proxy_mode = ProxyMode::Offline;
        ctx.extensions.insert(RoutingData {
            server_config: Arc::new(config),
            config_id: "lobby".to_string(),
            load_balancer: Arc::new(crate::loadbalancer::FirstAvailable),
        });
        ctx.extensions.insert(LoginData {
            username: "probe".to_string(),
            player_uuid: Some(UUID),
            profile_key: None,
            raw_payload: Bytes::from_static(PAYLOAD),
            layout,
        });
        if refused {
            ctx.extensions
                .insert(LayoutRefusal(ProtocolError::invalid("strict layout")));
        }
        (ctx, client)
    }

    fn listening(
        bus: &Arc<EventBusImpl>,
        decide: impl Fn(&mut LoginClaimEvent) + Send + Sync + 'static,
    ) -> Arc<Mutex<Vec<Seen>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let listeners: &dyn EventBus = &**bus;
        listeners.subscribe::<LoginClaimEvent, _>(EventPriority::NORMAL, move |event| {
            record.lock().unwrap().push(Seen {
                remote_addr: event.remote_addr,
                virtual_host: event.virtual_host.clone(),
                raw_host: event.raw_host.clone(),
                port: event.port,
                protocol: event.protocol_version.raw(),
                server: event.server.clone(),
                username: event.username.clone(),
                uuid: event.uuid,
                login_start: event.login_start.clone(),
                layout_matches: event.layout_matches,
            });
            decide(event);
        });
        seen
    }

    fn no_bans() -> Arc<BanManager> {
        Arc::new(BanManager::disabled(
            Arc::new(ConnectionRegistry::new()),
            Arc::new(EventBusImpl::new()),
        ))
    }

    async fn bans(dir: &tempfile::TempDir, target: BanTarget) -> Arc<BanManager> {
        let bans = Arc::new(BanManager::builtin(
            Arc::new(FileBanStorage::new(dir.path().join("bans.json"))),
            Arc::new(ConnectionRegistry::new()),
            Arc::new(EventBusImpl::new()),
        ));
        bans.issue(BanRequest::new(target).reason("griefing"))
            .await
            .unwrap();
        bans
    }

    fn middleware(bus: Arc<EventBusImpl>) -> LoginClaimMiddleware {
        LoginClaimMiddleware::new(bus, no_bans())
    }

    async fn claimed_by(
        bans: Arc<BanManager>,
    ) -> (Result<MiddlewareResult, CoreError>, ConnectionContext) {
        let bus = Arc::new(EventBusImpl::new());
        listening(&bus, |event| {
            event.handoff(Handoff::to(HandoffTarget::server_port("lobby", 30037)));
        });
        let (mut ctx, _client) = claim_ctx(LoginStartLayout::Declared, false).await;
        let result = LoginClaimMiddleware::new(bus, bans).process(&mut ctx).await;
        (result, ctx)
    }

    #[tokio::test]
    async fn a_claim_from_a_banned_address_is_kicked_like_a_banned_login() {
        let dir = tempfile::tempdir().unwrap();
        let bans = bans(&dir, BanTarget::Ip(IpAddr::V4(Ipv4Addr::LOCALHOST))).await;
        let (result, ctx) = claimed_by(bans).await;

        match result.unwrap() {
            MiddlewareResult::Kick(message) => {
                assert!(message.to_plain().contains("griefing"), "{message:?}");
            }
            other => panic!("a banned address must not be handed off, got {other:?}"),
        }
        assert_eq!(
            ctx.extensions.get::<Refused>(),
            Some(&Refused(RejectReason::IpBanned))
        );
        assert!(!ctx.extensions.contains::<ClaimedLogin>());
    }

    #[tokio::test]
    async fn a_claim_is_refused_while_the_ban_provider_cannot_answer() {
        let bans = Arc::new(BanManager::plugin(
            "absent",
            Arc::new(ConnectionRegistry::new()),
            Arc::new(EventBusImpl::new()),
        ));
        let (result, ctx) = claimed_by(bans).await;

        match result.unwrap() {
            MiddlewareResult::Kick(message) => {
                assert_eq!(message.to_plain(), BAN_CHECK_UNAVAILABLE);
            }
            other => panic!("an unchecked address must not be handed off, got {other:?}"),
        }
        assert_eq!(
            ctx.extensions.get::<Refused>(),
            Some(&Refused(RejectReason::Banned))
        );
        assert!(!ctx.extensions.contains::<ClaimedLogin>());
    }

    #[tokio::test]
    async fn only_the_address_is_checked_before_a_claim_is_handed_off() {
        let dir = tempfile::tempdir().unwrap();
        let bans = bans(&dir, BanTarget::Username("probe".into())).await;
        let (result, ctx) = claimed_by(bans).await;

        assert!(matches!(result.unwrap(), MiddlewareResult::ShortCircuit));
        assert!(ctx.extensions.contains::<ClaimedLogin>());
        assert!(!ctx.extensions.contains::<Refused>());
    }

    #[tokio::test]
    async fn an_unheard_login_goes_on_without_building_the_event() {
        let bus = Arc::new(EventBusImpl::new());
        let (mut ctx, _client) = claim_ctx(LoginStartLayout::Declared, false).await;
        let result = middleware(bus).process(&mut ctx).await;
        assert!(matches!(result.unwrap(), MiddlewareResult::Continue));
        assert!(!ctx.extensions.contains::<ClaimedLogin>());
    }

    #[tokio::test]
    async fn a_claim_carries_the_login_and_short_circuits_the_pipeline() {
        let bus = Arc::new(EventBusImpl::new());
        let handoff = Handoff::to(HandoffTarget::server_port("lobby", 30037));
        let decided = handoff.clone();
        let seen = listening(&bus, move |event| event.handoff(decided.clone()));
        let (mut ctx, _client) = claim_ctx(LoginStartLayout::Mismatched, true).await;
        let remote_addr = ctx.client_addr();

        let result = middleware(bus).process(&mut ctx).await;

        assert!(matches!(result.unwrap(), MiddlewareResult::ShortCircuit));
        assert_eq!(
            ctx.extensions.get::<ClaimedLogin>(),
            Some(&ClaimedLogin(handoff))
        );
        assert!(!ctx.extensions.contains::<LayoutRefusal>());
        assert_eq!(
            *seen.lock().unwrap(),
            [Seen {
                remote_addr,
                virtual_host: Some("lobby.test".to_string()),
                raw_host: "Lobby.Test\0FML3\0".to_string(),
                port: 25577,
                protocol: 763,
                server: Some(ServerId::new("lobby")),
                username: "probe".to_string(),
                uuid: Some(UUID),
                login_start: Bytes::from_static(PAYLOAD),
                layout_matches: false,
            }]
        );
    }

    #[tokio::test]
    async fn an_unclaimed_mismatched_layout_is_refused_with_the_strict_error() {
        let bus = Arc::new(EventBusImpl::new());
        let seen = listening(&bus, LoginClaimEvent::pass);
        let (mut ctx, _client) = claim_ctx(LoginStartLayout::Mismatched, true).await;

        let result = middleware(bus).process(&mut ctx).await;

        match result {
            Err(CoreError::Protocol(error)) => {
                assert_eq!(error.to_string(), "invalid: strict layout");
            }
            other => panic!("an unclaimed mismatched layout must be refused, got {other:?}"),
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(!ctx.extensions.contains::<ClaimedLogin>());
    }

    #[tokio::test]
    async fn a_held_refusal_applies_even_once_the_listener_is_gone() {
        let bus = Arc::new(EventBusImpl::new());
        let (mut ctx, _client) = claim_ctx(LoginStartLayout::Mismatched, true).await;
        let result = middleware(bus).process(&mut ctx).await;
        assert!(matches!(result, Err(CoreError::Protocol(_))));
    }

    #[tokio::test]
    async fn a_passed_declared_login_goes_on() {
        let bus = Arc::new(EventBusImpl::new());
        let fired = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&fired);
        listening(&bus, move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let (mut ctx, _client) = claim_ctx(LoginStartLayout::Declared, false).await;
        let result = middleware(bus).process(&mut ctx).await;
        assert!(matches!(result.unwrap(), MiddlewareResult::Continue));
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }
}
