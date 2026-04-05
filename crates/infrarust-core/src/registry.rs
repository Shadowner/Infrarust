//! Thread-safe registry of active proxy sessions.

use std::net::IpAddr;
use std::sync::Arc;

use dashmap::DashMap;
use infrarust_api::player::Player;
use uuid::Uuid;

use crate::player::PlayerSession;

/// Thread-safe registry of active proxy sessions.
///
/// Pure data structure backed by `DashMap` — no background tasks.
/// Handlers call `register()` at start, `unregister()` at end.
pub struct ConnectionRegistry {
    sessions: DashMap<Uuid, Arc<PlayerSession>>,
}

impl ConnectionRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Registers a player session, keyed by profile UUID.
    pub fn register(&self, session: Arc<PlayerSession>) -> Uuid {
        let uuid = session.profile().uuid;
        if let Some(previous) = self.sessions.insert(uuid, Arc::clone(&session)) {
            previous.shutdown_token().cancel();
            previous.set_disconnected();
            tracing::warn!(
                uuid = %uuid,
                username = %session.profile().username,
                "replaced existing session for UUID; previous session was cancelled"
            );
        }
        uuid
    }

    /// Removes a session by UUID, marking it as disconnected.
    pub fn unregister(&self, session_uuid: &Uuid) -> Option<Arc<PlayerSession>> {
        self.sessions.remove(session_uuid).map(|(_, session)| {
            session.set_disconnected();
            session
        })
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

impl infrarust_server_manager::PlayerCounter for ConnectionRegistry {
    fn count_by_server(&self, server_id: &str) -> usize {
        self.count_by_server(server_id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::player::PlayerCommand;
    use infrarust_api::types::{GameProfile, PlayerId, ServerId};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn make_session(username: &str, server: &str) -> Arc<PlayerSession> {
        let (tx, _rx) = mpsc::channel::<PlayerCommand>(32);
        Arc::new(PlayerSession::new(
            PlayerId::new(0),
            GameProfile {
                uuid: Uuid::new_v4(),
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
        ))
    }

    #[test]
    fn register_and_get() {
        let registry = ConnectionRegistry::new();
        let session = make_session("alice", "lobby");
        let uuid = session.profile().uuid;
        registry.register(session);
        let found = registry.get(&uuid).unwrap();
        assert_eq!(found.profile().username, "alice");
    }

    #[test]
    fn unregister_removes() {
        let registry = ConnectionRegistry::new();
        let session = make_session("bob", "survival");
        let uuid = session.profile().uuid;
        registry.register(session);
        assert!(registry.unregister(&uuid).is_some());
        assert!(registry.get(&uuid).is_none());
    }

    #[test]
    fn find_by_username() {
        let registry = ConnectionRegistry::new();
        registry.register(make_session("alice", "lobby"));
        registry.register(make_session("bob", "survival"));
        let found = registry.find_by_username("bob").unwrap();
        assert_eq!(found.current_server().unwrap().as_str(), "survival");
        assert!(registry.find_by_username("charlie").is_none());
    }

    #[test]
    fn count_by_server() {
        let registry = ConnectionRegistry::new();
        registry.register(make_session("alice", "lobby"));
        registry.register(make_session("bob", "lobby"));
        registry.register(make_session("charlie", "survival"));
        assert_eq!(registry.count(), 3);
        assert_eq!(registry.count_by_server("lobby"), 2);
        assert_eq!(registry.count_by_server("survival"), 1);
        assert_eq!(registry.count_by_server("creative"), 0);
    }

    #[test]
    fn concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let registry = Arc::new(ConnectionRegistry::new());
        let mut handles = vec![];

        for i in 0..10 {
            let reg = Arc::clone(&registry);
            handles.push(thread::spawn(move || {
                let session = make_session(&format!("player_{i}"), "lobby");
                reg.register(session);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(registry.count(), 10);
    }
}
