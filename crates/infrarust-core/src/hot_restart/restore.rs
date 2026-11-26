use std::collections::HashSet;
use std::fmt;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use infrarust_api::player::Player;
use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion, ServerId};
use infrarust_config::{ProxyMode, ServerAddress};
use infrarust_transport::{FrozenRelay, Listener};
use tracing::Instrument;
use uuid::Uuid;

use super::freeze;
use super::package::{
    DroppedSession, FrozenSession, Handle, HandoverPackage, ListenerHandover, UnpackFailure,
    Unpacked,
};
use super::resume::ResumeRecord;
use super::state::SessionMeta;
use crate::handler::forwarded::{ForwardedSession, RelayEnd, Relayed, resume_restored};
use crate::pipeline::context::reserve_connection_ids;
use crate::player::lifecycle::PlayerLifecycle;
use crate::player::{PlayerSession, SessionKind};
use crate::server::{ProxyServer, ProxyServices};

#[derive(Debug)]
pub struct Inherited {
    from_version: String,
    frozen_at: SystemTime,
    next_player_id: u64,
    listener: Option<ListenerHandover>,
    sessions: Vec<FrozenSession>,
    dropped: Vec<DroppedSession>,
    held: Vec<Handle>,
    resumes: Vec<ResumeRecord>,
}

impl Inherited {
    pub fn unpack(package: HandoverPackage) -> Result<Self, UnpackFailure> {
        let Unpacked {
            from_version,
            created_at_ms,
            next_player_id,
            listener,
            sessions,
            dropped,
            held,
            resumes,
        } = package.unpack()?;
        Ok(Self {
            from_version,
            frozen_at: UNIX_EPOCH + Duration::from_millis(created_at_ms),
            next_player_id,
            listener,
            sessions,
            dropped,
            held,
            resumes,
        })
    }

    pub fn from_version(&self) -> &str {
        &self.from_version
    }

    pub fn previous_bind(&self) -> Option<SocketAddr> {
        self.listener.as_ref().map(|listener| listener.bind)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn take_listener(&mut self, bind: SocketAddr) -> Option<TcpListener> {
        if self.previous_bind()? != bind {
            return None;
        }
        self.take_previous_listener()
    }

    pub fn take_previous_listener(&mut self) -> Option<TcpListener> {
        self.listener.take().map(|listener| listener.socket)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error(
        "the proxy already holds {players} players and {relays} relays; inherited sessions are only restored before it serves anything"
    )]
    NotFresh { players: usize, relays: usize },
}

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct PrepareFailure {
    pub error: RestoreError,
    pub inherited: Box<Inherited>,
}

#[derive(Debug)]
pub struct Prepared {
    from_version: String,
    frozen_at: SystemTime,
    next_player_id: u64,
    revivals: Vec<Revival>,
    rejected: Vec<Rejected>,
    dropped: Vec<DroppedSession>,
    stale_listener: Option<TcpListener>,
    held: Vec<Handle>,
    resumes: Vec<ResumeRecord>,
}

impl Prepared {
    pub fn resumable(&self) -> usize {
        self.revivals.len()
    }

    pub fn lost(&self) -> usize {
        self.rejected.len() + self.dropped.len()
    }
}

#[derive(Debug)]
struct Revival {
    meta: SessionMeta,
    connected_address: Option<ServerAddress>,
    relay: FrozenRelay,
}

#[derive(Debug)]
struct Rejected {
    session: FrozenSession,
    reason: Rejection,
}

#[derive(Debug, Clone, Copy)]
enum Rejection {
    NotForwarding(ProxyMode),
    DuplicatePlayer,
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotForwarding(mode) => write!(f, "{mode:?} sessions are not handed over"),
            Self::DuplicatePlayer => {
                f.write_str("another inherited session has the same player id")
            }
        }
    }
}

#[derive(Debug)]
pub struct Restored {
    pub players: Vec<Arc<PlayerSession>>,
    pub dropped: usize,
    pub frozen_for: Duration,
}

pub fn prepare(server: &ProxyServer, inherited: Inherited) -> Result<Prepared, PrepareFailure> {
    let players = server.registry().count();
    let relays = server.relays().active();
    if players > 0 || relays > 0 {
        return Err(PrepareFailure {
            error: RestoreError::NotFresh { players, relays },
            inherited: Box::new(inherited),
        });
    }

    let Inherited {
        from_version,
        frozen_at,
        mut next_player_id,
        listener,
        sessions,
        dropped,
        held,
        resumes,
    } = inherited;
    let router = &server.services().domain_router;
    let mut seen = HashSet::with_capacity(sessions.len());
    let mut revivals = Vec::with_capacity(sessions.len());
    let mut rejected = Vec::new();
    for session in sessions {
        let mode = session.meta.mode;
        let rejection = if !mode.is_forwarding() {
            Some(Rejection::NotForwarding(mode))
        } else if !seen.insert(session.meta.player_id) {
            Some(Rejection::DuplicatePlayer)
        } else {
            None
        };
        if let Some(reason) = rejection {
            rejected.push(Rejected { session, reason });
            continue;
        }
        let FrozenSession { meta, relay } = session;
        next_player_id = next_player_id.max(meta.player_id.saturating_add(1));
        if router.find_by_server_id(&meta.server).is_none() {
            tracing::warn!(
                player = %meta.profile.username,
                server = %meta.server,
                "an inherited session relays to a server the new configuration does not know, it keeps its live backend connection"
            );
        }
        revivals.push(Revival {
            connected_address: connected_address(&meta),
            meta,
            relay,
        });
    }

    Ok(Prepared {
        from_version,
        frozen_at,
        next_player_id,
        revivals,
        rejected,
        dropped,
        stale_listener: listener.map(|listener| listener.socket),
        held,
        resumes,
    })
}

pub fn launch(server: &ProxyServer, listener: &Listener, prepared: Prepared) -> Restored {
    let lost = prepared.lost();
    let Prepared {
        from_version,
        frozen_at,
        next_player_id,
        revivals,
        rejected,
        dropped,
        stale_listener,
        held,
        resumes,
    } = prepared;
    let waiting = server.services().resumes.admit(resumes);
    if waiting > 0 {
        tracing::info!(
            players = waiting,
            "players sent back by the previous image are expected to reconnect"
        );
    }
    reserve_connection_ids(next_player_id);
    drop(stale_listener);
    drop(held);
    for Rejected { session, reason } in rejected {
        tracing::warn!(
            player = %session.meta.profile.username,
            %reason,
            "closing an inherited session this proxy cannot resume"
        );
    }
    for session in &dropped {
        tracing::warn!(
            player = %session.meta.profile.username,
            error = %session.error,
            "an inherited session was lost in the handover"
        );
    }

    let services = server.services();
    let mut players = Vec::with_capacity(revivals.len());
    for revival in revivals {
        let slot = listener.reserve(revival.meta.client_addr.ip());
        let (player, session, relay) = revive(services, revival);
        let uuid = player.profile().uuid;
        tracing::debug!(session = %uuid, server = %session.server, "session resumed");
        let span = tracing::info_span!("proxy.session", mode = ?session.mode, restored = true);
        #[cfg(feature = "telemetry")]
        let metrics = SessionMetrics::resumed(server, &player, &session.server);
        let services = services.clone();
        server.track_session(
            async move {
                let relayed = resume_restored(&services, session, relay).await;
                #[cfg(feature = "telemetry")]
                if !matches!(relayed.reason, RelayEnd::Frozen) {
                    metrics.ended();
                }
                drop(slot);
                report(uuid, &relayed);
            }
            .instrument(span),
        );
        players.push(player);
    }

    let frozen_for = SystemTime::now()
        .duration_since(frozen_at)
        .unwrap_or_default();
    tracing::info!(
        from = %from_version,
        restored = players.len(),
        dropped = lost,
        frozen_ms = u64::try_from(frozen_for.as_millis()).unwrap_or(u64::MAX),
        "inherited sessions resumed"
    );
    Restored {
        players,
        dropped: lost,
        frozen_for,
    }
}

fn connected_address(meta: &SessionMeta) -> Option<ServerAddress> {
    let address = meta.connected_address.as_deref()?;
    match address.parse() {
        Ok(address) => Some(address),
        Err(error) => {
            tracing::warn!(
                player = %meta.profile.username,
                address,
                %error,
                "an inherited session has a backend address that cannot be read, it is not counted for load balancing"
            );
            None
        }
    }
}

fn revive(
    services: &ProxyServices,
    revival: Revival,
) -> (Arc<PlayerSession>, ForwardedSession, FrozenRelay) {
    let Revival {
        meta,
        connected_address,
        relay,
    } = revival;
    let connected_at = freeze::connected_at(&meta);
    let SessionMeta {
        player_id,
        profile,
        protocol_version,
        mode,
        client_addr,
        server,
        virtual_host,
        ..
    } = meta;
    let (command_tx, commands) = PlayerSession::channel();
    let builder = PlayerSession::builder(
        PlayerId::new(player_id),
        GameProfile::from(profile),
        ProtocolVersion::new(protocol_version),
        client_addr,
        command_tx,
        services.relays.sessions().child_token(),
        Arc::clone(&services.backend_load),
    )
    .kind(SessionKind::Forwarded)
    .permissions(Arc::clone(&services.permission_service))
    .events(Arc::clone(&services.event_bus))
    .connected_at(connected_at);
    let player = match virtual_host {
        Some(host) => builder.virtual_host(host),
        None => builder,
    }
    .build();

    let lifecycle = PlayerLifecycle::restore(services, Arc::clone(&player));
    let server = ServerId::new(server);
    player.set_connected_address(connected_address);
    player.set_current_server(server.clone());
    let session = ForwardedSession {
        player: Arc::clone(&player),
        lifecycle,
        commands,
        server,
        mode,
    };
    (player, session, relay)
}

#[cfg(feature = "telemetry")]
struct SessionMetrics {
    metrics: Arc<crate::telemetry::ProxyMetrics>,
    server: String,
    connected_at: SystemTime,
}

#[cfg(feature = "telemetry")]
impl SessionMetrics {
    fn resumed(server: &ProxyServer, player: &PlayerSession, server_id: &ServerId) -> Self {
        let metrics = Arc::clone(server.metrics());
        metrics.record_connection_resumed(server_id.as_str());
        Self {
            metrics,
            server: server_id.as_str().to_string(),
            connected_at: player.connected_at(),
        }
    }

    fn ended(self) {
        let lasted = SystemTime::now()
            .duration_since(self.connected_at)
            .unwrap_or_default();
        crate::handler::helpers::record_session_end(
            &Some(self.metrics),
            lasted,
            &self.server,
            "passthrough",
        );
    }
}

fn report(session: Uuid, relayed: &Relayed) {
    let (c2b, b2c) = (relayed.client_to_backend, relayed.backend_to_client);
    match &relayed.reason {
        RelayEnd::Ended(reason) => {
            tracing::info!(%session, c2b, b2c, ?reason, "session ended");
        }
        RelayEnd::Frozen => {
            tracing::debug!(%session, c2b, b2c, "session frozen for a hot restart");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::Path;
    use std::sync::Mutex;

    use bytes::Bytes;
    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::events::lifecycle::{DisconnectEvent, PostLoginEvent};
    use infrarust_config::ProxyConfig;
    use infrarust_transport::FrozenDirection;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::hot_restart::memory::{InheritanceOutcome, MemoryInheritance};
    use crate::hot_restart::platform::Inheritance;
    use crate::hot_restart::state::ProfileRecord;
    use crate::pipeline::context::connection_id_watermark;

    const T: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(100);

    type Log = Arc<Mutex<Vec<String>>>;

    async fn server(dir: &Path) -> ProxyServer {
        let servers_dir = dir.join("servers");
        std::fs::create_dir(&servers_dir).unwrap();
        let mut config: ProxyConfig = toml::from_str("bind = \"127.0.0.1:0\"\n").unwrap();
        config.servers_dir = servers_dir;
        config.plugins_dir = dir.join("plugins");
        config.ban.file = dir.join("bans.json");
        ProxyServer::new(config, dir.join("infrarust.toml"), CancellationToken::new())
            .await
            .unwrap()
    }

    fn record(server: &ProxyServer) -> Log {
        let log = Log::default();
        let bus: &dyn EventBus = server.services().event_bus.as_ref();
        let seen = Arc::clone(&log);
        bus.subscribe::<PostLoginEvent, _>(EventPriority::LAST, move |event| {
            seen.lock()
                .unwrap()
                .push(format!("post_login:{}", event.player_id().as_u64()));
        });
        let seen = Arc::clone(&log);
        bus.subscribe::<DisconnectEvent, _>(EventPriority::LAST, move |event| {
            seen.lock().unwrap().push(format!(
                "disconnect:{}:{}",
                event.player_id().as_u64(),
                event.cause.as_str()
            ));
        });
        log
    }

    struct Peers {
        player: TcpStream,
        server: TcpStream,
    }

    fn pair(listener: &TcpListener) -> (TcpStream, std::net::TcpStream) {
        let outer = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (inner, _) = listener.accept().unwrap();
        outer.set_nonblocking(true).unwrap();
        (TcpStream::from_std(outer).unwrap(), inner)
    }

    fn session(
        player_id: u64,
        mode: ProxyMode,
        to_server: &'static [u8],
        to_player: &'static [u8],
    ) -> (FrozenSession, Peers) {
        let front = TcpListener::bind("127.0.0.1:0").unwrap();
        let back = TcpListener::bind("127.0.0.1:0").unwrap();
        let (player, client) = pair(&front);
        let (server, backend) = pair(&back);
        let server_addr = back.local_addr().unwrap();
        let meta = SessionMeta {
            player_id,
            profile: ProfileRecord {
                uuid: Uuid::from_u128(u128::from(player_id)),
                username: format!("player{player_id}"),
                properties: Vec::new(),
            },
            protocol_version: 767,
            mode,
            client_addr: player.local_addr().unwrap(),
            server: "lobby".into(),
            connected_address: Some(server_addr.to_string()),
            virtual_host: Some("lobby.test".into()),
            connected_at_ms: 1_700_000_000_123,
        };
        let relay = FrozenRelay {
            client,
            backend,
            client_to_backend: FrozenDirection {
                tail: Bytes::from_static(to_server),
                source_eof: false,
                copied: 40,
            },
            backend_to_client: FrozenDirection {
                tail: Bytes::from_static(to_player),
                source_eof: false,
                copied: 80,
            },
            first_closed: None,
        };
        (FrozenSession { meta, relay }, Peers { player, server })
    }

    fn package(
        next_player_id: u64,
        listener: Option<(TcpListener, &str)>,
        sessions: Vec<FrozenSession>,
    ) -> HandoverPackage {
        let listener = listener.map(|(socket, bind)| ListenerHandover {
            socket,
            bind: bind.parse().unwrap(),
        });
        let (package, dropped) =
            HandoverPackage::build("2.0.0-beta.6", next_player_id, listener, sessions).unwrap();
        assert!(dropped.is_empty());
        package
    }

    async fn expect(stream: &mut TcpStream, bytes: &[u8]) {
        let mut buf = vec![0; bytes.len()];
        tokio::time::timeout(T, stream.read_exact(&mut buf))
            .await
            .expect("the bytes must arrive")
            .unwrap();
        assert_eq!(buf, bytes);
    }

    async fn stays_open(stream: &mut TcpStream) {
        let mut buf = [0; 1];
        assert!(
            tokio::time::timeout(QUIET, stream.read(&mut buf))
                .await
                .is_err(),
            "the socket must neither close nor carry data"
        );
    }

    async fn closes(stream: &mut TcpStream) {
        let mut buf = [0; 1];
        let read = tokio::time::timeout(T, stream.read(&mut buf))
            .await
            .expect("the socket must close");
        assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    }

    async fn relays_both_ways(peers: &mut Peers) {
        peers.player.write_all(b"hello").await.unwrap();
        expect(&mut peers.server, b"hello").await;
        peers.server.write_all(b"world").await.unwrap();
        expect(&mut peers.player, b"world").await;
    }

    async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let waited = tokio::time::timeout(T, async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(waited.is_ok(), "timed out waiting for {what}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inherited_sessions_resume_their_tails_first_and_keep_relaying() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path()).await;
        let log = record(&server);
        let (steve, mut steve_peers) =
            session(4100, ProxyMode::Passthrough, b"steve-up", b"steve-down");
        let (alex, mut alex_peers) = session(4101, ProxyMode::ZeroCopy, b"alex-up", b"alex-down");
        let accept = TcpListener::bind("127.0.0.1:0").unwrap();
        let accept_addr = accept.local_addr().unwrap();
        let mut sent = package(4102, Some((accept, "127.0.0.1:0")), vec![steve, alex]);
        sent.state.created_at_ms -= 1_500;
        let mut inheritance = MemoryInheritance::new(sent);
        let outcome = inheritance.record();

        let mut inherited = Inherited::unpack(inheritance.take_package().unwrap()).unwrap();
        assert_eq!(inherited.from_version(), "2.0.0-beta.6");
        assert_eq!(inherited.session_count(), 2);
        let adopted = inherited
            .take_listener(server.services().config.bind)
            .expect("a listener on the configured address is adopted");
        let listener = server.bind_or_adopt(Some(adopted)).await.unwrap();
        assert_eq!(listener.local_addr(), accept_addr);
        let prepared = prepare(&server, inherited).unwrap();
        assert_eq!((prepared.resumable(), prepared.lost()), (2, 0));
        Box::new(inheritance).commit().unwrap();
        let restored = launch(&server, &listener, prepared);

        assert_eq!(outcome.outcome(), InheritanceOutcome::Committed);
        assert_eq!(restored.players.len(), 2);
        assert_eq!(restored.dropped, 0);
        assert!(restored.frozen_for >= Duration::from_millis(1_500));
        assert!(connection_id_watermark() >= 4102);
        expect(&mut steve_peers.server, b"steve-up").await;
        expect(&mut steve_peers.player, b"steve-down").await;
        expect(&mut alex_peers.server, b"alex-up").await;
        expect(&mut alex_peers.player, b"alex-down").await;
        relays_both_ways(&mut steve_peers).await;
        relays_both_ways(&mut alex_peers).await;

        let steve = server.registry().find_by_id(PlayerId::new(4100)).unwrap();
        assert_eq!(steve.profile().username, "player4100");
        assert_eq!(steve.protocol_version(), ProtocolVersion::new(767));
        assert_eq!(
            steve.remote_addr(),
            steve_peers.player.local_addr().unwrap()
        );
        assert_eq!(steve.current_server(), Some(ServerId::new("lobby")));
        assert_eq!(
            steve.connected_address(),
            Some(
                steve_peers
                    .server
                    .peer_addr()
                    .unwrap()
                    .to_string()
                    .parse()
                    .unwrap()
            )
        );
        assert_eq!(steve.virtual_host().as_deref(), Some("lobby.test"));
        assert_eq!(
            steve.connected_at(),
            UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)
        );
        assert_eq!(server.registry().count(), 2);
        assert_eq!(server.relays().active(), 2);
        assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());

        let Peers {
            player,
            server: mut backend,
        } = steve_peers;
        drop(player);
        closes(&mut backend).await;
        drop(backend);
        wait_until("Steve's session to end", || server.registry().count() == 1).await;
        wait_until("Steve's disconnect event", || {
            log.lock()
                .unwrap()
                .iter()
                .any(|line| line == "disconnect:4100:client_quit")
        })
        .await;
        relays_both_ways(&mut alex_peers).await;
        assert_eq!(server.relays().active(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_preparation_hands_every_socket_back() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path()).await;
        let busy = server.relays().ticket();
        let (steve, mut peers) = session(4300, ProxyMode::Passthrough, b"", b"");
        let accept = TcpListener::bind("127.0.0.1:0").unwrap();
        let accept_addr = accept.local_addr().unwrap();
        let inherited =
            Inherited::unpack(package(4301, Some((accept, "127.0.0.1:0")), vec![steve])).unwrap();

        let failure = prepare(&server, inherited).unwrap_err();

        assert!(
            matches!(
                failure.error,
                RestoreError::NotFresh {
                    players: 0,
                    relays: 1
                }
            ),
            "{failure}"
        );
        assert!(failure.to_string().contains("1 relays"), "{failure}");
        assert_eq!(failure.inherited.session_count(), 1);
        assert_eq!(
            failure.inherited.previous_bind(),
            Some("127.0.0.1:0".parse().unwrap())
        );
        stays_open(&mut peers.player).await;
        stays_open(&mut peers.server).await;
        std::net::TcpStream::connect(accept_addr).unwrap();

        drop(failure);

        closes(&mut peers.player).await;
        closes(&mut peers.server).await;
        drop(busy);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sessions_that_cannot_resume_stay_open_until_launch() {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path()).await;
        let (kept, mut kept_peers) = session(4400, ProxyMode::ServerOnly, b"", b"");
        let (offline, mut offline_peers) = session(4401, ProxyMode::Offline, b"", b"");
        let (twin, mut twin_peers) = session(4400, ProxyMode::Passthrough, b"", b"");
        let (broken, mut broken_peers) = session(4402, ProxyMode::Passthrough, b"", b"");
        let stale = TcpListener::bind("127.0.0.1:0").unwrap();
        let stale_addr = stale.local_addr().unwrap();
        let mut sent = package(
            10,
            Some((stale, "127.0.0.1:0")),
            vec![kept, offline, twin, broken],
        );
        let swapped = &mut sent.state.sessions[3].backend;
        std::mem::swap(&mut swapped.local, &mut swapped.peer);
        let mut inherited = Inherited::unpack(sent).unwrap();
        let moved: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(inherited.take_listener(moved).is_none());
        let listener = server.bind().await.unwrap();

        let prepared = prepare(&server, inherited).unwrap();

        assert_eq!((prepared.resumable(), prepared.lost()), (1, 3));
        for peers in [&mut offline_peers, &mut twin_peers, &mut broken_peers] {
            stays_open(&mut peers.player).await;
            stays_open(&mut peers.server).await;
        }
        std::net::TcpStream::connect(stale_addr).unwrap();

        let restored = launch(&server, &listener, prepared);

        assert_eq!(restored.players.len(), 1);
        assert_eq!(restored.players[0].id(), PlayerId::new(4400));
        assert_eq!(restored.dropped, 3);
        assert!(connection_id_watermark() >= 4401);
        for peers in [&mut offline_peers, &mut twin_peers, &mut broken_peers] {
            closes(&mut peers.player).await;
            closes(&mut peers.server).await;
        }
        assert!(std::net::TcpStream::connect(stale_addr).is_err());
        relays_both_ways(&mut kept_peers).await;
        assert_eq!(server.registry().count(), 1);
    }

    #[test]
    fn the_inherited_listener_is_adopted_only_while_the_configured_bind_is_unchanged() {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let real = socket.local_addr().unwrap();
        let mut inherited =
            Inherited::unpack(package(1, Some((socket, "0.0.0.0:25565")), Vec::new())).unwrap();
        assert_eq!(
            inherited.previous_bind(),
            Some("0.0.0.0:25565".parse().unwrap())
        );

        assert!(
            inherited
                .take_listener("0.0.0.0:25566".parse().unwrap())
                .is_none()
        );
        assert!(inherited.take_listener(real).is_none());
        let adopted = inherited
            .take_listener("0.0.0.0:25565".parse().unwrap())
            .expect("an unchanged bind adopts the listener whatever its real address");
        assert_eq!(adopted.local_addr().unwrap(), real);
        assert!(
            inherited
                .take_listener("0.0.0.0:25565".parse().unwrap())
                .is_none()
        );
        assert_eq!(inherited.previous_bind(), None);
    }
}
