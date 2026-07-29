//! Thread-safe registry of active proxy sessions.

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
}

impl ConnectionRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            id_index: DashMap::new(),
        }
    }

    /// Registers a player session, keyed by profile UUID.
    ///
    /// The returned guard unregisters the session when dropped.
    pub fn register(self: &Arc<Self>, session: Arc<PlayerSession>) -> SessionGuard {
        let uuid = session.profile().uuid;
        let player_id = session.id();
        self.id_index.insert(player_id, uuid);
        if let Some(previous) = self.sessions.insert(uuid, Arc::clone(&session)) {
            if previous.id() != player_id {
                self.id_index.remove(&previous.id());
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
        self.id_index.remove_if(&player_id, |_, u| u == session_uuid);
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

    /// Finds the first session matching the given username.
    pub fn find_by_username(&self, username: &str) -> Option<Arc<PlayerSession>> {
        self.sessions
            .iter()
            .find(|r| r.profile().username == username)
            .map(|r| Arc::clone(&r))
    }

    /// Returns all sessions connected to the given server.
    pub fn find_by_server(&self, server_id: &str) -> Vec<Arc<PlayerSession>> {
        self.sessions
            .iter()
            .filter(|r| r.current_server().is_some_and(|s| s.as_str() == server_id))
            .map(|r| Arc::clone(&r))
            .collect()
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    pub fn count_by_server(&self, server_id: &str) -> usize {
        self.sessions
            .iter()
            .filter(|r| r.current_server().is_some_and(|s| s.as_str() == server_id))
            .count()
    }

    /// Returns a snapshot of all active sessions.
    pub fn all(&self) -> Vec<Arc<PlayerSession>> {
        self.sessions.iter().map(|r| Arc::clone(&r)).collect()
    }

    /// Finds all sessions from a given IP (may be multiple for multi-accounts).
    pub fn find_by_ip(&self, ip: &IpAddr) -> Vec<Arc<PlayerSession>> {
        self.sessions
            .iter()
            .filter(|r| r.remote_addr().ip() == *ip)
            .map(|r| Arc::clone(&r))
            .collect()
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
        assert_eq!(registry.find_by_id(PlayerId::new(2)).unwrap().id().as_u64(), 2);
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
