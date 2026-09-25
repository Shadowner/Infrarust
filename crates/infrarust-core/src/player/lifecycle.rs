use std::sync::Arc;
use std::time::Duration;

use infrarust_api::events::lifecycle::{DisconnectCause, DisconnectEvent, PostLoginEvent};
use infrarust_api::player::Player;
use infrarust_api::types::Component;

use super::PlayerSession;
use crate::event_bus::EventBusImpl;
use crate::registry::SessionGuard;
use crate::services::ProxyServices;

pub(crate) const REPLACED_REASON: &str = "You logged in from another location";

pub(crate) struct PlayerLifecycle {
    bus: Arc<EventBusImpl>,
    deadline: Duration,
    registration: Option<Registration>,
}

struct Registration {
    session: Arc<PlayerSession>,
    guard: Option<SessionGuard>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        drop(self.guard.take());
        self.session.mark_released();
    }
}

impl PlayerLifecycle {
    pub(crate) async fn begin(services: &ProxyServices, session: Arc<PlayerSession>) -> Self {
        let deadline = services.config.events.disconnect_deadline;
        displace_previous(services, &session, deadline).await;
        let guard = services.connection_registry.register(Arc::clone(&session));
        let lifecycle = Self {
            bus: Arc::clone(&services.event_bus),
            deadline,
            registration: Some(Registration {
                session: Arc::clone(&session),
                guard: Some(guard),
            }),
        };
        let _ = services
            .event_bus
            .fire(PostLoginEvent::new(session as Arc<dyn Player>))
            .await;
        lifecycle
    }

    pub(crate) async fn end(mut self, cause: DisconnectCause) {
        if let Some(registration) = self.registration.take() {
            finish(Arc::clone(&self.bus), registration, cause, self.deadline).await;
        }
    }
}

impl Drop for PlayerLifecycle {
    fn drop(&mut self) {
        let Some(registration) = self.registration.take() else {
            return;
        };
        tracing::debug!(
            player = %registration.session.profile().username,
            "player session ended without a cause, reporting it as an error"
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(finish(
                Arc::clone(&self.bus),
                registration,
                DisconnectCause::Error,
                self.deadline,
            ));
        }
    }
}

async fn finish(
    bus: Arc<EventBusImpl>,
    registration: Registration,
    cause: DisconnectCause,
    deadline: Duration,
) {
    let session = Arc::clone(&registration.session);
    let event = DisconnectEvent::new(
        Arc::clone(&session) as Arc<dyn Player>,
        session.current_server(),
        cause,
    );
    let dispatch = tokio::spawn(async move {
        let _ = bus.fire(event).await;
    });
    let abort = dispatch.abort_handle();
    if tokio::time::timeout(deadline, dispatch).await.is_err() {
        abort.abort();
        tracing::warn!(
            player = %session.profile().username,
            ?deadline,
            "DisconnectEvent listeners did not finish within [events] disconnect_deadline, releasing the player without them"
        );
    }
    drop(registration);
}

async fn displace_previous(services: &ProxyServices, session: &PlayerSession, deadline: Duration) {
    let uuid = session.profile().uuid;
    let give_up = tokio::time::Instant::now() + deadline;
    while let Some(previous) = services.connection_registry.find_by_uuid(&uuid) {
        if previous.id() == session.id() {
            return;
        }
        tracing::info!(
            %uuid,
            username = %session.profile().username,
            "profile logged in again, disconnecting its previous session"
        );
        previous.disconnect(Component::text(REPLACED_REASON)).await;
        if tokio::time::timeout_at(give_up, previous.released())
            .await
            .is_err()
        {
            tracing::warn!(
                %uuid,
                ?deadline,
                "previous session did not end within [events] disconnect_deadline, replacing it"
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::permissions::{PermissionChecker, PermissionLevel};
    use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion, ServerId};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::auth::game_profile::offline_uuid;
    use crate::limbo::test_helpers::test_proxy_services;
    use crate::player::PlayerCommand;

    type Log = Arc<Mutex<Vec<String>>>;

    fn services_with_deadline(deadline: Duration) -> ProxyServices {
        let mut services = test_proxy_services();
        let mut config = (*services.config).clone();
        config.events.disconnect_deadline = deadline;
        services.config = Arc::new(config);
        services
    }

    fn player(
        services: &ProxyServices,
        id: u64,
        username: &str,
    ) -> (Arc<PlayerSession>, mpsc::Receiver<PlayerCommand>) {
        let (tx, rx) = PlayerSession::channel();
        let session = PlayerSession::new(
            PlayerId::new(id),
            GameProfile {
                uuid: offline_uuid(username),
                username: username.to_string(),
                properties: vec![],
            },
            ProtocolVersion::new(767),
            "127.0.0.1:40000".parse().unwrap(),
            None,
            true,
            false,
            tx,
            CancellationToken::new(),
            crate::permissions::default_checker(),
            Arc::clone(&services.backend_load),
        );
        (Arc::new(session), rx)
    }

    fn record(services: &ProxyServices) -> Log {
        let log = Log::default();
        let bus: &dyn EventBus = services.event_bus.as_ref();
        let registry = Arc::clone(&services.connection_registry);
        let seen = Arc::clone(&log);
        bus.subscribe::<PostLoginEvent, _>(EventPriority::LAST, move |event| {
            let registered = registry.find_by_id(event.player_id()).is_some();
            seen.lock().unwrap().push(format!(
                "post_login:{}:{registered}:{:?}",
                event.player_id().as_u64(),
                event.player.current_server()
            ));
        });
        let seen = Arc::clone(&log);
        bus.subscribe::<DisconnectEvent, _>(EventPriority::LAST, move |event| {
            seen.lock().unwrap().push(format!(
                "disconnect:{}:{}:{}",
                event.player_id().as_u64(),
                event.cause.as_str(),
                event.last_server.as_ref().map_or("-", ServerId::as_str)
            ));
        });
        log
    }

    fn lines(log: &Log) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn post_login_runs_once_the_player_is_registered() {
        let services = test_proxy_services();
        let log = record(&services);
        let (steve, _rx) = player(&services, 1, "Steve");

        let lifecycle = PlayerLifecycle::begin(&services, Arc::clone(&steve)).await;

        assert_eq!(lines(&log), ["post_login:1:true:None"]);
        lifecycle.end(DisconnectCause::ClientQuit).await;
    }

    #[tokio::test]
    async fn end_reports_the_current_server_then_releases_the_player() {
        let services = test_proxy_services();
        let log = record(&services);
        let (steve, _rx) = player(&services, 1, "Steve");
        let lifecycle = PlayerLifecycle::begin(&services, Arc::clone(&steve)).await;
        steve.set_current_server(ServerId::new("lobby"));
        steve.set_current_server(ServerId::new("survival"));

        lifecycle.end(DisconnectCause::ClientQuit).await;

        assert_eq!(
            lines(&log),
            [
                "post_login:1:true:None",
                "disconnect:1:client_quit:survival"
            ]
        );
        assert_eq!(services.connection_registry.count(), 0);
        assert!(!steve.is_connected());
        tokio::time::timeout(Duration::from_secs(1), steve.released())
            .await
            .expect("the player must be released");
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_disconnect_listener_is_cut_off_at_the_deadline() {
        let services = services_with_deadline(Duration::from_millis(300));
        let bus: &dyn EventBus = services.event_bus.as_ref();
        bus.subscribe_async::<DisconnectEvent, _>(EventPriority::NORMAL, |_| {
            Box::pin(std::future::pending())
        });
        let (steve, _rx) = player(&services, 1, "Steve");
        let lifecycle = PlayerLifecycle::begin(&services, Arc::clone(&steve)).await;
        let started = tokio::time::Instant::now();

        lifecycle.end(DisconnectCause::ClientQuit).await;

        assert_eq!(started.elapsed(), Duration::from_millis(300));
        assert_eq!(services.connection_registry.count(), 0);
    }

    #[tokio::test]
    async fn dropping_without_a_cause_reports_an_error() {
        let services = test_proxy_services();
        let log = record(&services);
        let (steve, _rx) = player(&services, 1, "Steve");
        let lifecycle = PlayerLifecycle::begin(&services, Arc::clone(&steve)).await;

        drop(lifecycle);

        tokio::time::timeout(Duration::from_secs(5), steve.released())
            .await
            .expect("the dropped lifecycle must still release the player");
        assert_eq!(
            lines(&log),
            ["post_login:1:true:None", "disconnect:1:error:-"]
        );
        assert_eq!(services.connection_registry.count(), 0);
    }

    #[tokio::test]
    async fn a_second_login_waits_for_the_first_to_end() {
        let services = test_proxy_services();
        let log = record(&services);
        let (first, mut first_commands) = player(&services, 1, "Steve");
        let first_lifecycle = PlayerLifecycle::begin(&services, Arc::clone(&first)).await;
        let first_session = Arc::clone(&first);
        let first_flow = tokio::spawn(async move {
            first_session.shutdown_token().cancelled().await;
            let reason = match first_commands.recv().await {
                Some(PlayerCommand::Kick(reason)) => Some(reason),
                _ => None,
            };
            first_lifecycle
                .end(DisconnectCause::Kicked {
                    reason: reason.clone(),
                })
                .await;
            reason
        });
        let (second, _rx) = player(&services, 2, "Steve");

        let second_lifecycle = PlayerLifecycle::begin(&services, Arc::clone(&second)).await;

        assert_eq!(
            first_flow.await.unwrap(),
            Some(Component::text(REPLACED_REASON))
        );
        assert_eq!(
            lines(&log),
            [
                "post_login:1:true:None",
                "disconnect:1:kicked:-",
                "post_login:2:true:None"
            ]
        );
        assert_eq!(
            services
                .connection_registry
                .find_by_uuid(&offline_uuid("Steve"))
                .map(|p| p.id()),
            Some(PlayerId::new(2))
        );
        second_lifecycle.end(DisconnectCause::ClientQuit).await;
    }

    struct Admin;

    impl PermissionChecker for Admin {
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::Admin
        }

        fn has_permission(&self, _permission: &str) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn the_permission_checker_can_change_after_the_session_is_built() {
        let services = test_proxy_services();
        let (steve, _rx) = player(&services, 1, "Steve");
        assert!(!steve.has_permission("infrarust.admin"));
        assert_eq!(steve.permission_level(), PermissionLevel::Player);

        steve.set_permission_checker(Arc::new(Admin));

        assert!(steve.has_permission("infrarust.admin"));
        assert_eq!(steve.permission_level(), PermissionLevel::Admin);
    }

    fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    #[test]
    fn post_login_events_are_only_built_here() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let needles = [
            concat!("PostLoginEvent", "::new("),
            concat!("PostLoginEvent", " {"),
        ];
        let mut builders: Vec<String> = files
            .iter()
            .filter(|path| {
                let text = std::fs::read_to_string(path).unwrap();
                needles.iter().any(|needle| text.contains(needle))
            })
            .map(|path| {
                path.strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        builders.sort();

        assert_eq!(builders, ["player/lifecycle.rs"]);
    }
}
