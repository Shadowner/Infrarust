//! Thread-safe registry of active proxy sessions.

use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Arc;

use dashmap::DashMap;
use infrarust_api::player::Player;
use infrarust_api::types::PlayerId;
use uuid::Uuid;

use crate::player::PlayerSession;

/// Thread-safe registry of active proxy sessions.
///
/// Pure data structure backed by `DashMap` — no background tasks.
/// Handlers call `register()` at start, `unregister()` at end.
pub struct ConnectionRegistry {
    sessions: DashMap<Uuid, Arc<PlayerSession>>,
    id_index: DashMap<PlayerId, Uuid>,
    name_index: SessionIndex<String>,
    ip_index: SessionIndex<IpAddr>,
}

struct SessionIndex<K> {
    entries: DashMap<K, Vec<Arc<PlayerSession>>>,
}

impl<K: Eq + Hash> SessionIndex<K> {
    fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    fn insert(&self, key: K, session: &Arc<PlayerSession>) {
        self.entries
            .entry(key)
            .or_default()
            .push(Arc::clone(session));
    }

    fn remove(&self, key: &K, player_id: PlayerId) {
        self.entries.remove_if_mut(key, |_, sessions| {
            sessions.retain(|s| s.id() != player_id);
            sessions.is_empty()
        });
    }
}

fn name_key(username: &str) -> String {
    username.to_lowercase()
}

impl ConnectionRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            id_index: DashMap::new(),
            name_index: SessionIndex::new(),
            ip_index: SessionIndex::new(),
        }
    }

    fn index(&self, session: &Arc<PlayerSession>) {
        self.name_index
            .insert(name_key(&session.profile().username), session);
        self.ip_index
            .insert(session.remote_addr().ip().to_canonical(), session);
    }

    fn unindex(&self, session: &PlayerSession) {
        self.name_index
            .remove(&name_key(&session.profile().username), session.id());
        self.ip_index
            .remove(&session.remote_addr().ip().to_canonical(), session.id());
    }

    /// Registers a player session, keyed by profile UUID.
    ///
    /// The returned guard unregisters the session when dropped.
    pub fn register(self: &Arc<Self>, session: Arc<PlayerSession>) -> SessionGuard {
        let uuid = session.profile().uuid;
        let player_id = session.id();
        self.id_index.insert(player_id, uuid);
        self.index(&session);
        if let Some(previous) = self.sessions.insert(uuid, Arc::clone(&session)) {
            if previous.id() != player_id {
                self.id_index.remove(&previous.id());
                self.unindex(&previous);
            }
            previous.shutdown_token().cancel();
            previous.set_disconnected();
            tracing::warn!(
                uuid = %uuid,
                username = %session.profile().username,
                "replaced existing session for UUID; previous session was cancelled"
            );
        }
        SessionGuard {
            registry: Arc::clone(self),
            uuid,
            player_id,
        }
    }

    /// Removes a session, marking it as disconnected.
    ///
    /// The `player_id` check is what makes this safe against UUID collisions:
    /// a session replaced by [`register`](Self::register) must not be evicted
    /// by the cleanup of the session it replaced.
    fn unregister(&self, session_uuid: &Uuid, player_id: PlayerId) -> Option<Arc<PlayerSession>> {
        let (_, session) = self
            .sessions
            .remove_if(session_uuid, |_, s| s.id() == player_id)?;
        self.id_index
            .remove_if(&player_id, |_, u| u == session_uuid);
        self.unindex(&session);
        session.set_disconnected();
        Some(session)
    }

    pub fn find_by_id(&self, id: PlayerId) -> Option<Arc<PlayerSession>> {
        let uuid = *self.id_index.get(&id)?;
        self.get(&uuid)
    }

    /// Returns a reference-counted handle to the session.
    pub fn get(&self, session_uuid: &Uuid) -> Option<Arc<PlayerSession>> {
        self.sessions.get(session_uuid).map(|r| Arc::clone(&r))
    }

    pub fn find_by_username(&self, username: &str) -> Option<Arc<PlayerSession>> {
        let matches = self.name_index.entries.get(&name_key(username))?;
        matches
            .iter()
            .find(|s| s.profile().username == username)
            .or_else(|| matches.first())
            .map(Arc::clone)
    }

    pub fn find_by_server(&self, server_id: &str) -> Vec<Arc<PlayerSession>> {
        self.sessions
            .iter()
            .filter(|r| r.counted_server().is_some_and(|s| s.as_str() == server_id))
            .map(|r| Arc::clone(&r))
            .collect()
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    pub fn count_by_server(&self, server_id: &str) -> usize {
        self.sessions
            .iter()
            .filter(|r| r.counted_server().is_some_and(|s| s.as_str() == server_id))
            .count()
    }

    /// Returns a snapshot of all active sessions.
    pub fn all(&self) -> Vec<Arc<PlayerSession>> {
        self.sessions.iter().map(|r| Arc::clone(&r)).collect()
    }

    /// Finds all sessions from a given IP (may be multiple for multi-accounts).
    pub fn find_by_ip(&self, ip: &IpAddr) -> Vec<Arc<PlayerSession>> {
        self.ip_index
            .entries
            .get(&ip.to_canonical())
            .map(|sessions| sessions.clone())
            .unwrap_or_default()
    }

    /// Finds the session with the given Mojang UUID.
    ///
    /// Delegates to [`get()`](Self::get) — both are keyed by UUID.
    pub fn find_by_uuid(&self, uuid: &Uuid) -> Option<Arc<PlayerSession>> {
        self.get(uuid)
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use = "dropping the guard unregisters the session"]
pub struct SessionGuard {
    registry: Arc<ConnectionRegistry>,
    uuid: Uuid,
    player_id: PlayerId,
}

impl SessionGuard {
    pub const fn uuid(&self) -> Uuid {
        self.uuid
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.registry.unregister(&self.uuid, self.player_id);
    }
}

impl infrarust_server_manager::PlayerCounter for ConnectionRegistry {
    fn count_by_server(&self, server_id: &str) -> usize {
        self.count_by_server(server_id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::loadbalancer::{AddressConnectionCount, BackendLoad};
    use crate::player::PlayerCommand;
    use infrarust_api::types::{GameProfile, PlayerId, ServerId};
    use infrarust_config::ServerAddress;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn session(
        id: u64,
        uuid: Uuid,
        username: &str,
        server: &str,
        load: &Arc<BackendLoad>,
    ) -> Arc<PlayerSession> {
        let (tx, _rx) = mpsc::channel::<PlayerCommand>(32);
        Arc::new(PlayerSession::new(
            PlayerId::new(id),
            GameProfile {
                uuid,
                username: username.to_string(),
                properties: vec![],
            },
            infrarust_api::types::ProtocolVersion::new(767),
            "127.0.0.1:12345".parse().unwrap(),
            Some(ServerId::new(server)),
            false,
            false,
            tx,
            CancellationToken::new(),
            crate::permissions::default_checker(),
            Arc::clone(load),
        ))
    }

    fn connected(id: u64, uuid: Uuid, username: &str, addr: &str) -> Arc<PlayerSession> {
        let (tx, _rx) = mpsc::channel::<PlayerCommand>(32);
        Arc::new(PlayerSession::new(
            PlayerId::new(id),
            GameProfile {
                uuid,
                username: username.to_string(),
                properties: vec![],
            },
            infrarust_api::types::ProtocolVersion::new(767),
            addr.parse().unwrap(),
            None,
            true,
            false,
            tx,
            CancellationToken::new(),
            crate::permissions::default_checker(),
            Arc::new(BackendLoad::new()),
        ))
    }

    fn ids(sessions: &[Arc<PlayerSession>]) -> Vec<u64> {
        let mut ids: Vec<u64> = sessions.iter().map(|s| s.id().as_u64()).collect();
        ids.sort_unstable();
        ids
    }

    fn found_id(registry: &ConnectionRegistry, username: &str) -> Option<u64> {
        registry.find_by_username(username).map(|s| s.id().as_u64())
    }

    fn is_empty(registry: &ConnectionRegistry) -> bool {
        registry.sessions.is_empty()
            && registry.id_index.is_empty()
            && registry.name_index.entries.is_empty()
            && registry.ip_index.entries.is_empty()
    }

    fn make_session_with_id(id: u64, username: &str, server: &str) -> Arc<PlayerSession> {
        session(
            id,
            Uuid::new_v4(),
            username,
            server,
            &Arc::new(BackendLoad::new()),
        )
    }

    fn make_session(username: &str, server: &str) -> Arc<PlayerSession> {
        make_session_with_id(0, username, server)
    }

    fn addr(host: &str) -> ServerAddress {
        ServerAddress {
            host: host.to_string(),
            port: 25565,
        }
    }

    #[test]
    fn register_and_get() {
        let registry = Arc::new(ConnectionRegistry::new());
        let session = make_session("alice", "lobby");
        let uuid = session.profile().uuid;
        let _guard = registry.register(session);
        let found = registry.get(&uuid).unwrap();
        assert_eq!(found.profile().username, "alice");
    }

    #[test]
    fn dropping_the_guard_removes() {
        let registry = Arc::new(ConnectionRegistry::new());
        let session = make_session("bob", "survival");
        let uuid = session.profile().uuid;
        drop(registry.register(Arc::clone(&session)));
        assert!(registry.get(&uuid).is_none());
        assert!(!session.is_connected());
    }

    #[test]
    fn replaced_session_survives_previous_cleanup() {
        let registry = Arc::new(ConnectionRegistry::new());
        let first = make_session_with_id(1, "alice", "lobby");
        let uuid = first.profile().uuid;
        let first_guard = registry.register(first);

        // Fast reconnect: same profile UUID, new session, replaces the first.
        let second = session(2, uuid, "alice", "lobby", &Arc::new(BackendLoad::new()));
        let _second_guard = registry.register(Arc::clone(&second));

        drop(first_guard);

        assert_eq!(registry.get(&uuid).map(|s| s.id()), Some(PlayerId::new(2)));
        assert!(second.is_connected());
        assert_eq!(
            registry.find_by_id(PlayerId::new(2)).unwrap().id().as_u64(),
            2
        );
    }

    #[test]
    fn find_by_username() {
        let registry = Arc::new(ConnectionRegistry::new());
        let _a = registry.register(make_session("alice", "lobby"));
        let _b = registry.register(make_session("bob", "survival"));
        let found = registry.find_by_username("bob").unwrap();
        assert_eq!(found.current_server().unwrap().as_str(), "survival");
        assert!(registry.find_by_username("charlie").is_none());
    }

    #[test]
    fn find_by_id_is_correct_and_cleaned_up() {
        let registry = Arc::new(ConnectionRegistry::new());
        let _alice = registry.register(make_session_with_id(1, "alice", "lobby"));
        let bob_guard = registry.register(make_session_with_id(2, "bob", "survival"));

        let found = registry.find_by_id(PlayerId::new(2)).unwrap();
        assert_eq!(found.profile().username, "bob");
        assert!(registry.find_by_id(PlayerId::new(99)).is_none());

        drop(bob_guard);
        assert!(registry.find_by_id(PlayerId::new(2)).is_none());
        assert!(registry.find_by_id(PlayerId::new(1)).is_some());
    }

    #[test]
    fn count_by_server() {
        let registry = Arc::new(ConnectionRegistry::new());
        let _a = registry.register(make_session("alice", "lobby"));
        let _b = registry.register(make_session("bob", "lobby"));
        let _c = registry.register(make_session("charlie", "survival"));
        assert_eq!(registry.count(), 3);
        assert_eq!(registry.count_by_server("lobby"), 2);
        assert_eq!(registry.count_by_server("survival"), 1);
        assert_eq!(registry.count_by_server("creative"), 0);
    }

    #[test]
    fn a_pending_initial_server_counts_until_the_player_joins_one() {
        let registry = Arc::new(ConnectionRegistry::new());
        let (held, _rx) = PlayerSession::new_test(true);
        let held = Arc::new(held);
        let _guard = registry.register(Arc::clone(&held));
        assert_eq!(registry.count_by_server("lobby"), 0);

        held.set_pending_server(ServerId::new("lobby"));

        assert_eq!(held.current_server(), None);
        assert_eq!(registry.count_by_server("lobby"), 1);
        assert_eq!(registry.find_by_server("lobby").len(), 1);

        held.set_current_server(ServerId::new("survival"));

        assert_eq!(registry.count_by_server("lobby"), 0);
        assert_eq!(registry.count_by_server("survival"), 1);
        assert!(registry.find_by_server("lobby").is_empty());
    }

    #[test]
    fn per_address_count_lifecycle() {
        let registry = Arc::new(ConnectionRegistry::new());
        let load = Arc::new(BackendLoad::new());
        let (a, b) = (addr("10.0.0.1"), addr("10.0.0.2"));
        let alice = session(1, Uuid::new_v4(), "alice", "lobby", &load);
        let guard = registry.register(Arc::clone(&alice));
        assert_eq!(load.active_connections_for_address(&a), 0);

        // Initial connect
        alice.set_connected_address(Some(a.clone()));
        assert_eq!(load.active_connections_for_address(&a), 1);

        // Server switch
        alice.set_connected_address(Some(b.clone()));
        assert_eq!(load.active_connections_for_address(&a), 0);
        assert_eq!(load.active_connections_for_address(&b), 1);

        // Limbo: connected to no backend address
        alice.set_connected_address(None);
        assert_eq!(load.active_connections_for_address(&b), 0);

        // Limbo exit then disconnect
        alice.set_connected_address(Some(a.clone()));
        assert_eq!(load.active_connections_for_address(&a), 1);
        drop(guard);
        assert_eq!(load.active_connections_for_address(&a), 0);
    }

    #[test]
    fn replace_path_releases_previous_address() {
        let registry = Arc::new(ConnectionRegistry::new());
        let load = Arc::new(BackendLoad::new());
        let a = addr("10.0.0.1");
        let uuid = Uuid::new_v4();

        let first = session(1, uuid, "alice", "lobby", &load);
        let first_guard = registry.register(Arc::clone(&first));
        first.set_connected_address(Some(a.clone()));

        let second = session(2, uuid, "alice", "lobby", &load);
        let _second_guard = registry.register(Arc::clone(&second));
        second.set_connected_address(Some(a.clone()));

        // The replaced session is disconnected by register, releasing its slot.
        assert_eq!(load.active_connections_for_address(&a), 1);
        drop(first_guard);
        assert_eq!(load.active_connections_for_address(&a), 1);
    }

    #[test]
    fn dropped_session_releases_its_address() {
        let load = Arc::new(BackendLoad::new());
        let a = addr("10.0.0.1");
        let orphan = session(1, Uuid::new_v4(), "alice", "lobby", &load);
        orphan.set_connected_address(Some(a.clone()));
        assert_eq!(load.active_connections_for_address(&a), 1);
        drop(orphan);
        assert_eq!(load.active_connections_for_address(&a), 0);
    }

    #[test]
    fn a_username_is_found_in_any_case() {
        let registry = Arc::new(ConnectionRegistry::new());
        let _steve = registry.register(connected(1, Uuid::new_v4(), "Steve", "10.0.0.1:1"));

        for spelling in ["Steve", "steve", "STEVE", "sTeVe"] {
            assert_eq!(found_id(&registry, spelling), Some(1), "{spelling}");
        }
        assert_eq!(found_id(&registry, "Stev"), None);
        assert_eq!(found_id(&registry, "Steven"), None);
    }

    #[test]
    fn an_exact_spelling_wins_over_another_case() {
        let registry = Arc::new(ConnectionRegistry::new());
        let _upper = registry.register(connected(1, Uuid::new_v4(), "Steve", "10.0.0.1:1"));
        let lower = registry.register(connected(2, Uuid::new_v4(), "steve", "10.0.0.2:1"));

        assert_eq!(found_id(&registry, "Steve"), Some(1));
        assert_eq!(found_id(&registry, "steve"), Some(2));
        assert!(found_id(&registry, "STEVE").is_some());

        drop(lower);
        assert_eq!(found_id(&registry, "steve"), Some(1));
    }

    #[test]
    fn unregistering_clears_every_index() {
        let registry = Arc::new(ConnectionRegistry::new());
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let alice = registry.register(connected(1, Uuid::new_v4(), "Alice", "10.0.0.1:1"));
        let bob = registry.register(connected(2, Uuid::new_v4(), "Bob", "10.0.0.1:2"));
        assert_eq!(ids(&registry.find_by_ip(&ip)), [1, 2]);

        drop(alice);

        assert_eq!(found_id(&registry, "alice"), None);
        assert_eq!(found_id(&registry, "bob"), Some(2));
        assert_eq!(ids(&registry.find_by_ip(&ip)), [2]);

        drop(bob);

        assert!(registry.find_by_ip(&ip).is_empty());
        assert!(is_empty(&registry));
    }

    #[test]
    fn a_replaced_session_leaves_the_indexes_to_its_successor() {
        let registry = Arc::new(ConnectionRegistry::new());
        let uuid = Uuid::new_v4();
        let old_ip: IpAddr = "10.0.0.1".parse().unwrap();
        let new_ip: IpAddr = "10.0.0.2".parse().unwrap();
        let first = registry.register(connected(1, uuid, "Alice", "10.0.0.1:1"));

        let second = registry.register(connected(2, uuid, "ALICE", "10.0.0.2:1"));

        assert_eq!(found_id(&registry, "alice"), Some(2));
        assert!(registry.find_by_ip(&old_ip).is_empty());
        assert_eq!(ids(&registry.find_by_ip(&new_ip)), [2]);

        drop(first);

        assert_eq!(found_id(&registry, "Alice"), Some(2));
        assert_eq!(ids(&registry.find_by_ip(&new_ip)), [2]);
        assert_eq!(
            registry.find_by_uuid(&uuid).map(|s| s.id().as_u64()),
            Some(2)
        );

        drop(second);

        assert!(is_empty(&registry));
    }

    #[test]
    fn an_ipv4_mapped_address_is_the_same_client() {
        let registry = Arc::new(ConnectionRegistry::new());
        let _mapped = registry.register(connected(
            1,
            Uuid::new_v4(),
            "Alice",
            "[::ffff:203.0.113.7]:1",
        ));

        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        assert_eq!(ids(&registry.find_by_ip(&ip)), [1]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_logins_keep_the_indexes_consistent() {
        let registry = Arc::new(ConnectionRegistry::new());
        let shared_uuid = Uuid::new_v4();
        let mut tasks = Vec::new();
        for i in 0..64u64 {
            let registry = Arc::clone(&registry);
            tasks.push(tokio::spawn(async move {
                let (uuid, name) = if i % 4 == 0 {
                    (shared_uuid, "Shared".to_string())
                } else {
                    (Uuid::new_v4(), format!("Player{i}"))
                };
                let spelling = if i % 2 == 0 {
                    name.to_uppercase()
                } else {
                    name.to_lowercase()
                };
                let addr = format!("10.0.{}.1:{}", i % 3, 1000 + i);
                let guard = registry.register(connected(i + 1, uuid, &spelling, &addr));
                tokio::task::yield_now().await;
                let found = registry.find_by_username(&name);
                if uuid != shared_uuid {
                    assert_eq!(found.map(|s| s.id().as_u64()), Some(i + 1));
                }
                tokio::task::yield_now().await;
                drop(guard);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        assert!(is_empty(&registry));
    }

    #[test]
    fn concurrent_access() {
        use std::thread;

        let registry = Arc::new(ConnectionRegistry::new());
        let mut handles = vec![];

        for i in 0..10 {
            let reg = Arc::clone(&registry);
            handles.push(thread::spawn(move || {
                reg.register(make_session(&format!("player_{i}"), "lobby"))
            }));
        }

        let _guards: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        assert_eq!(registry.count(), 10);
    }
}
