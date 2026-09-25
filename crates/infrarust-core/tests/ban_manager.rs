#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::error::ServiceError;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::services::ban_service::{
    BanFeatures, BanPage, BanProvider, BanProviderRejected, BanQuery, BanRequest, BanService,
    BanVerdict, LoginAttempt, UnbanRequest,
};
use infrarust_api::types::{Component, GameProfile, PlayerId, ProtocolVersion, ServerId};
use infrarust_core::ban::FileBanStorage;
use infrarust_core::ban::manager::BanManager;
use infrarust_core::ban::types::{BanEntry, BanSource, BanTarget};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::player::{PlayerCommand, PlayerSession};
use infrarust_core::registry::{ConnectionRegistry, SessionGuard};
use infrarust_core::services::ban_bridge::PluginBanService;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct World {
    manager: Arc<BanManager>,
    registry: Arc<ConnectionRegistry>,
    bus: Arc<EventBusImpl>,
    _dir: Option<tempfile::TempDir>,
}

fn bus() -> Arc<EventBusImpl> {
    let bus = Arc::new(EventBusImpl::new());
    bus.start_dispatcher();
    bus
}

fn builtin_world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(ConnectionRegistry::new());
    let bus = bus();
    let manager = Arc::new(BanManager::builtin(
        Arc::new(FileBanStorage::new(dir.path().join("bans.json"))),
        Arc::clone(&registry),
        Arc::clone(&bus),
    ));
    World {
        manager,
        registry,
        bus,
        _dir: Some(dir),
    }
}

fn plugin_world(plugin_id: &str) -> World {
    let registry = Arc::new(ConnectionRegistry::new());
    let bus = bus();
    let manager = Arc::new(BanManager::plugin(
        plugin_id,
        Arc::clone(&registry),
        Arc::clone(&bus),
    ));
    World {
        manager,
        registry,
        bus,
        _dir: None,
    }
}

struct Online {
    token: CancellationToken,
    commands: mpsc::Receiver<PlayerCommand>,
    _guard: SessionGuard,
}

impl Online {
    fn kick_reason(&mut self) -> Option<String> {
        match self.commands.try_recv() {
            Ok(PlayerCommand::Kick(reason)) => Some(reason.to_plain()),
            _ => None,
        }
    }
}

fn join(world: &World, username: &str, ip: &str) -> Online {
    let token = CancellationToken::new();
    let (tx, commands) = PlayerSession::channel();
    let uuid = Uuid::new_v4();
    let session = Arc::new(PlayerSession::new(
        PlayerId::new(uuid.as_u128() as u64),
        GameProfile {
            uuid,
            username: username.to_string(),
            properties: vec![],
        },
        ProtocolVersion::new(767),
        std::net::SocketAddr::new(ip.parse().unwrap(), 12345),
        Some(ServerId::new("lobby")),
        true,
        false,
        tx,
        token.clone(),
        infrarust_core::permissions::default_checker(),
        Arc::new(infrarust_core::loadbalancer::BackendLoad::new()),
    ));
    let guard = world.registry.register(session);
    Online {
        token,
        commands,
        _guard: guard,
    }
}

#[derive(Default)]
struct TableProvider {
    entries: Mutex<Vec<BanEntry>>,
    message: Option<Component>,
    ranges: bool,
}

impl BanProvider for TableProvider {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.target.matches(attempt))
            .cloned();
        let message = self.message.clone();
        Box::pin(async move {
            Ok(found.map(|entry| match message {
                Some(message) => BanVerdict::new(entry).message(message),
                None => BanVerdict::new(entry),
            }))
        })
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        let mut entries = self.entries.lock().unwrap();
        let mut entry = BanEntry::new(
            format!("t{}", entries.len() + 1),
            request.target,
            request.source.expect("the facade always names a source"),
        );
        entry.reason = request.reason;
        entries.push(entry.clone());
        Box::pin(async move { Ok(entry) })
    }

    fn unban(
        &self,
        request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        let mut entries = self.entries.lock().unwrap();
        let removed = entries
            .iter()
            .position(|entry| entry.target == request.target)
            .map(|index| entries.remove(index));
        Box::pin(async move { Ok(removed) })
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| &entry.target == target)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list(&self, _query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        let entries = self.entries.lock().unwrap().clone();
        Box::pin(async move { Ok(BanPage::new(entries, None)) })
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(self.ranges)
    }
}

fn record<E: infrarust_api::event::Event + Clone>(bus: &EventBusImpl) -> Arc<Mutex<Vec<E>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let bus: &dyn EventBus = bus;
    bus.subscribe::<E, _>(EventPriority::NORMAL, move |event| {
        sink.lock().unwrap().push(event.clone());
    });
    seen
}

#[tokio::test]
async fn a_ban_kicks_the_online_player_with_its_message() {
    let world = builtin_world();
    let mut victim = join(&world, "Victim", "192.168.1.50");
    let bystander = join(&world, "Bystander", "192.168.1.51");

    let issued = world
        .manager
        .issue(BanRequest::new(BanTarget::Username("victim".into())).reason("bad behavior"))
        .await
        .unwrap();

    assert_eq!(issued.kicked, 1);
    assert!(victim.token.is_cancelled());
    assert!(victim.kick_reason().unwrap().contains("bad behavior"));
    assert!(!bystander.token.is_cancelled());
}

#[tokio::test]
async fn a_ban_that_does_not_kick_leaves_the_player_online() {
    let world = builtin_world();
    let victim = join(&world, "Victim", "192.168.1.50");

    let issued = world
        .manager
        .issue(BanRequest::new(BanTarget::Username("Victim".into())).kick(false))
        .await
        .unwrap();

    assert_eq!(issued.kicked, 0);
    assert!(!victim.token.is_cancelled());
}

#[tokio::test]
async fn an_ip_range_ban_kicks_every_player_behind_it() {
    let world = builtin_world();
    let mut inside = [
        join(&world, "One", "10.20.0.1"),
        join(&world, "Two", "10.20.200.9"),
        join(&world, "Mapped", "::ffff:10.20.3.3"),
    ];
    let outside = join(&world, "Outside", "10.21.0.1");

    let issued = world
        .manager
        .issue(
            BanRequest::new(BanTarget::IpRange("10.20.0.0/16".parse().unwrap())).reason("botnet"),
        )
        .await
        .unwrap();

    assert_eq!(issued.kicked, 3);
    for player in &mut inside {
        assert!(player.token.is_cancelled());
        assert!(player.kick_reason().unwrap().contains("botnet"));
    }
    assert!(!outside.token.is_cancelled());
}

#[tokio::test]
async fn an_ip_ban_kicks_every_player_sharing_the_address() {
    let world = builtin_world();
    let players = [
        join(&world, "Player1", "10.0.0.5"),
        join(&world, "Player2", "10.0.0.5"),
        join(&world, "Player3", "::ffff:10.0.0.5"),
    ];

    let ip: IpAddr = "10.0.0.5".parse().unwrap();
    world
        .manager
        .issue(BanRequest::new(BanTarget::Ip(ip)).reason("shared IP"))
        .await
        .unwrap();

    for player in &players {
        assert!(player.token.is_cancelled());
    }
}

#[tokio::test]
async fn a_request_without_a_source_is_recorded_as_the_system() {
    let world = builtin_world();

    let entry = BanService::ban(
        world.manager.as_ref(),
        BanRequest::new(BanTarget::Username("Anon".into())),
    )
    .await
    .unwrap();

    assert_eq!(entry.source, BanSource::System);
}

#[tokio::test]
async fn a_plugin_ban_is_attributed_to_the_plugin_unless_it_names_a_source() {
    let world = builtin_world();
    let plugin = PluginBanService::new(Arc::clone(&world.manager) as Arc<dyn BanService>, "guard");

    let attributed = plugin
        .ban(BanRequest::new(BanTarget::Username("One".into())))
        .await
        .unwrap();
    let explicit = plugin
        .ban(
            BanRequest::new(BanTarget::Username("Two".into()))
                .source(BanSource::WebApi { actor: None }),
        )
        .await
        .unwrap();

    assert_eq!(attributed.source, BanSource::Plugin("guard".into()));
    assert_eq!(explicit.source, BanSource::WebApi { actor: None });
    assert_eq!(
        world
            .manager
            .get(&BanTarget::Username("one".into()))
            .await
            .unwrap()
            .unwrap()
            .source,
        BanSource::Plugin("guard".into())
    );
}

#[tokio::test]
async fn issued_and_revoked_bans_reach_listeners_in_order() {
    let world = builtin_world();
    let issued = record::<BanIssuedEvent>(&world.bus);
    let revoked = record::<BanRevokedEvent>(&world.bus);
    let plugin = PluginBanService::new(Arc::clone(&world.manager) as Arc<dyn BanService>, "mod");
    let target = BanTarget::Username("Griefer".into());

    let entry = world
        .manager
        .issue(
            BanRequest::new(target.clone())
                .source(BanSource::Console)
                .silent(true),
        )
        .await
        .unwrap()
        .entry;
    let removed = plugin
        .unban(UnbanRequest::new(target.clone()))
        .await
        .unwrap()
        .expect("the ban existed");
    let nothing = plugin.unban(UnbanRequest::new(target)).await.unwrap();
    world.bus.flush().await;

    assert_eq!(removed.id, entry.id);
    assert!(nothing.is_none());
    let issued = issued.lock().unwrap().clone();
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].entry.id, entry.id);
    assert_eq!(issued[0].source, BanSource::Console);
    assert!(issued[0].silent);
    let revoked = revoked.lock().unwrap().clone();
    assert_eq!(revoked.len(), 1, "unbanning nothing fires nothing");
    assert_eq!(revoked[0].entry.id, entry.id);
    assert_eq!(revoked[0].source, BanSource::Plugin("mod".into()));
    assert!(!revoked[0].silent);
}

#[tokio::test]
async fn disabled_bans_check_nobody_and_refuse_management() {
    let registry = Arc::new(ConnectionRegistry::new());
    let manager = BanManager::disabled(registry, bus());
    let attempt = LoginAttempt::pre_auth("10.0.0.1".parse().unwrap(), "Anyone");

    assert!(manager.check(&attempt).await.unwrap().is_none());
    assert!(manager.refusal(&attempt).await.is_none());
    for result in [
        manager
            .issue(BanRequest::new(BanTarget::Username("x".into())))
            .await
            .map(|_| ()),
        manager
            .revoke(UnbanRequest::new(BanTarget::Username("x".into())))
            .await
            .map(|_| ()),
        manager.list(BanQuery::new()).await.map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(ServiceError::Unavailable(_))),
            "{result:?}"
        );
    }
}

#[tokio::test]
async fn only_the_selected_plugin_may_provide_bans() {
    let world = plugin_world("guard");
    let attempt = LoginAttempt::pre_auth("10.0.0.1".parse().unwrap(), "Anyone");
    assert!(matches!(
        world.manager.check(&attempt).await,
        Err(ServiceError::Unavailable(_))
    ));

    let rejected = world
        .manager
        .register_provider("other", Arc::new(TableProvider::default()));
    assert_eq!(
        rejected,
        Err(BanProviderRejected::NotSelected {
            selected: "guard".into()
        })
    );

    world
        .manager
        .register_provider("guard", Arc::new(TableProvider::default()))
        .unwrap();
    assert!(world.manager.check(&attempt).await.unwrap().is_none());

    world.manager.unregister_provider("other");
    assert!(world.manager.check(&attempt).await.is_ok());
    world.manager.unregister_provider("guard");
    assert!(matches!(
        world.manager.check(&attempt).await,
        Err(ServiceError::Unavailable(_))
    ));
}

#[tokio::test]
async fn a_provider_without_ip_ranges_refuses_range_bans() {
    let world = plugin_world("guard");
    world
        .manager
        .register_provider("guard", Arc::new(TableProvider::default()))
        .unwrap();

    let result = world
        .manager
        .issue(BanRequest::new(BanTarget::IpRange(
            "10.0.0.0/8".parse().unwrap(),
        )))
        .await;

    assert!(matches!(result, Err(ServiceError::OperationFailed(_))));
}

#[tokio::test]
async fn a_kick_carries_the_providers_own_message() {
    let world = plugin_world("guard");
    world
        .manager
        .register_provider(
            "guard",
            Arc::new(TableProvider {
                message: Some(Component::text("Appeal at example.org")),
                ranges: true,
                ..TableProvider::default()
            }),
        )
        .unwrap();
    let mut victim = join(&world, "Victim", "203.0.113.9");

    world
        .manager
        .issue(BanRequest::new(BanTarget::IpRange(
            "203.0.113.0/24".parse().unwrap(),
        )))
        .await
        .unwrap();

    assert!(victim.token.is_cancelled());
    assert_eq!(
        victim.kick_reason().as_deref(),
        Some("Appeal at example.org")
    );
}

#[tokio::test]
async fn test_purge_task_stops_on_shutdown() {
    let world = builtin_world();
    let shutdown = CancellationToken::new();

    let handle = world
        .manager
        .start_purge_task(Duration::from_millis(50), shutdown.clone())
        .expect("the builtin provider purges");

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("purge task should stop within timeout")
        .expect("purge task should not panic");
}

#[tokio::test]
async fn only_the_builtin_provider_runs_a_purge_task() {
    let world = plugin_world("guard");
    assert!(
        world
            .manager
            .start_purge_task(Duration::from_millis(50), CancellationToken::new())
            .is_none()
    );
}
