use std::net::SocketAddr;
use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::lifecycle::{
    GameProfileRequestEvent, LoginEvent, LoginResult, PreLoginEvent, PreLoginResult,
};
use infrarust_api::player::Player;
use infrarust_api::services::ban_service::LoginAttempt;
use infrarust_api::types::{Component, GameProfile, PlayerId, ProtocolVersion, ServerId};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::pipeline::context::ConnectionContext;
use crate::player::lifecycle::PlayerLifecycle;
use crate::player::{PlayerCommand, PlayerSession, SessionKind};
use crate::services::ProxyServices;

pub(crate) struct Arrival {
    pub(crate) profile: GameProfile,
    pub(crate) protocol_version: ProtocolVersion,
    pub(crate) domain: String,
    pub(crate) origin: ServerId,
}

pub(crate) enum PreLogin {
    Denied(Component),
    Proceed(PreLoginResult),
}

pub(crate) async fn pre_login(
    services: &ProxyServices,
    profile: GameProfile,
    remote_addr: SocketAddr,
    protocol_version: ProtocolVersion,
    domain: &str,
) -> PreLogin {
    let event = services
        .event_bus
        .fire(PreLoginEvent::new(
            profile,
            remote_addr,
            protocol_version,
            domain.to_string(),
        ))
        .await;
    match event.result().clone() {
        PreLoginResult::Denied { reason } => PreLogin::Denied(reason),
        result => PreLogin::Proceed(result),
    }
}

pub(crate) struct Admitted {
    pub(crate) session: Arc<PlayerSession>,
    pub(crate) lifecycle: PlayerLifecycle,
    pub(crate) commands: mpsc::Receiver<PlayerCommand>,
    pub(crate) token: CancellationToken,
    pub(crate) rewritten: bool,
}

pub(crate) enum Admission {
    Admitted(Admitted),
    Refused(Component),
}

pub(crate) async fn admit(
    services: &ProxyServices,
    ctx: &ConnectionContext,
    shutdown: &CancellationToken,
    arrival: Arrival,
    kind: SessionKind,
) -> Admission {
    let bus = &services.event_bus;
    let remote_addr = ctx.client_addr();
    let online_mode = kind.online_mode();

    let request = bus
        .fire(GameProfileRequestEvent::new(
            arrival.profile,
            online_mode,
            remote_addr,
            Some(arrival.domain.clone()),
            arrival.protocol_version,
        ))
        .await;
    let rewritten = request.is_modified();
    let profile = request.profile;

    let attempt = LoginAttempt::post_auth(
        ctx.client_ip,
        profile.username.clone(),
        profile.uuid,
        online_mode,
    )
    .virtual_host(arrival.domain.clone())
    .server(arrival.origin);
    if let Some(reason) = services.ban_manager.refusal(&attempt).await {
        return Admission::Refused(reason);
    }

    let token = shutdown.child_token();
    let (command_tx, commands) = PlayerSession::channel();
    let session = PlayerSession::builder(
        PlayerId::new(ctx.connection_id),
        profile,
        arrival.protocol_version,
        remote_addr,
        command_tx,
        token.clone(),
        Arc::clone(&services.backend_load),
    )
    .kind(kind)
    .permissions(Arc::clone(&services.permission_service))
    .virtual_host(arrival.domain)
    .events(Arc::clone(bus))
    .build();

    session.setup_permissions(bus).await;

    let login = bus
        .fire(LoginEvent::new(
            Arc::clone(&session) as Arc<dyn Player>,
            online_mode,
        ))
        .await;
    if let LoginResult::Denied { reason } = login.result() {
        tracing::info!(username = %session.profile().username, "login denied by a plugin");
        return Admission::Refused(reason.clone());
    }

    let lifecycle = PlayerLifecycle::begin(services, Arc::clone(&session)).await;
    Admission::Admitted(Admitted {
        session,
        lifecycle,
        commands,
        token,
        rewritten,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Mutex;

    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::events::lifecycle::DisconnectCause;
    use infrarust_api::services::ban_service::BanRequest;
    use tokio::net::{TcpListener, TcpStream};

    use super::*;
    use crate::ban::FileBanStorage;
    use crate::ban::manager::BanManager;
    use crate::ban::types::BanTarget;
    use crate::limbo::test_helpers::{test_profile, test_proxy_services};

    async fn connection() -> (ConnectionContext, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let peer = TcpStream::connect(addr).await.unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let peer_addr = "203.0.113.9:40000".parse().unwrap();
        let ctx = ConnectionContext::new_for_test(stream, peer_addr, peer_addr.ip(), addr);
        (ctx, peer)
    }

    fn arrival() -> Arrival {
        Arrival {
            profile: test_profile(),
            protocol_version: ProtocolVersion::new(767),
            domain: "lobby.test".to_string(),
            origin: ServerId::new("lobby"),
        }
    }

    fn record_logins(services: &ProxyServices) -> Arc<Mutex<Vec<bool>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let bus: &dyn EventBus = services.event_bus.as_ref();
        bus.subscribe::<LoginEvent, _>(EventPriority::NORMAL, move |event: &mut LoginEvent| {
            sink.lock().unwrap().push(event.online_mode);
        });
        seen
    }

    #[tokio::test]
    async fn a_ban_refuses_the_player_before_any_login_event() {
        let dir = tempfile::tempdir().unwrap();
        let mut services = test_proxy_services();
        services.event_bus.start_dispatcher();
        services.ban_manager = Arc::new(BanManager::builtin(
            Arc::new(FileBanStorage::new(dir.path().join("bans.json"))),
            Arc::clone(&services.connection_registry),
            Arc::clone(&services.event_bus),
        ));
        services
            .ban_manager
            .issue(BanRequest::new(BanTarget::Username("LimboTester".into())).reason("nope"))
            .await
            .unwrap();
        let logins = record_logins(&services);
        let (ctx, _peer) = connection().await;

        let admission = admit(
            &services,
            &ctx,
            &CancellationToken::new(),
            arrival(),
            SessionKind::Intercepted { online_mode: true },
        )
        .await;

        let Admission::Refused(reason) = admission else {
            panic!("a banned player must be refused");
        };
        assert!(reason.to_plain().contains("nope"), "{reason:?}");
        assert!(logins.lock().unwrap().is_empty());
        assert_eq!(services.connection_registry.count(), 0);
    }

    async fn admitted_online_mode(kind: SessionKind) -> (bool, bool) {
        let services = test_proxy_services();
        let logins = record_logins(&services);
        let (ctx, _peer) = connection().await;
        let admission = admit(&services, &ctx, &CancellationToken::new(), arrival(), kind).await;
        let Admission::Admitted(admitted) = admission else {
            panic!("the player must be admitted");
        };
        let session_online = admitted.session.is_online_mode();
        admitted.lifecycle.end(DisconnectCause::ClientQuit).await;
        let seen = logins.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        (seen[0], session_online)
    }

    #[tokio::test]
    async fn the_login_event_reports_the_online_mode_of_the_session_kind() {
        assert_eq!(
            admitted_online_mode(SessionKind::Intercepted { online_mode: true }).await,
            (true, true)
        );
        assert_eq!(
            admitted_online_mode(SessionKind::Intercepted { online_mode: false }).await,
            (false, false)
        );
        assert_eq!(
            admitted_online_mode(SessionKind::Forwarded).await,
            (false, false)
        );
    }
}
