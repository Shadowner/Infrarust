use std::net::IpAddr;

use tokio::time::Instant;

use dashmap::DashMap;
use infrarust_config::ProxyMode;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// An active proxy session entry.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    /// Unique session identifier.
    pub session_id: Uuid,
    /// Player username (set after login start parsing).
    pub username: Option<String>,
    /// Player UUID (set after login start parsing, 1.20.2+).
    pub player_uuid: Option<Uuid>,
    /// Effective client IP.
    pub client_ip: IpAddr,
    /// Identifier of the target server config.
    pub server_id: String,
    /// Proxy mode used for this session.
    pub proxy_mode: ProxyMode,
    /// When the connection was accepted.
    pub connected_at: Instant,
    /// Token to signal shutdown of this session.
    pub shutdown_token: CancellationToken,
}

/// Thread-safe registry of active proxy sessions.
///
/// Pure data structure backed by `DashMap` — no background tasks.
/// Passthrough handler calls `register()` at start, `unregister()` at end.
pub struct ConnectionRegistry {
    sessions: DashMap<Uuid, SessionEntry>,
}

impl ConnectionRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Registers a new session, returning its UUID.
    pub fn register(&self, entry: SessionEntry) -> Uuid {
        let id = entry.session_id;
        self.sessions.insert(id, entry);
        id
    }

    /// Removes a session by ID, returning the entry if it existed.
    pub fn unregister(&self, session_id: &Uuid) -> Option<SessionEntry> {
        self.sessions.remove(session_id).map(|(_, v)| v)
    }

    /// Returns a clone of the session entry for the given ID.
    pub fn get(&self, session_id: &Uuid) -> Option<SessionEntry> {
        self.sessions.get(session_id).map(|r| r.clone())
    }

    /// Finds the first session matching the given username.
    pub fn find_by_username(&self, username: &str) -> Option<SessionEntry> {
        self.sessions
            .iter()
            .find(|r| r.username.as_deref() == Some(username))
            .map(|r| r.clone())
    }

    /// Returns all sessions connected to the given server.
    pub fn find_by_server(&self, server_id: &str) -> Vec<SessionEntry> {
        self.sessions
            .iter()
            .filter(|r| r.server_id == server_id)
            .map(|r| r.clone())
            .collect()
    }

    /// Returns the total number of active sessions.
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// Returns the number of sessions connected to the given server.
    pub fn count_by_server(&self, server_id: &str) -> usize {
        self.sessions
            .iter()
            .filter(|r| r.server_id == server_id)
            .count()
    }

    /// Returns a snapshot of all active sessions.
    pub fn all(&self) -> Vec<SessionEntry> {
        self.sessions.iter().map(|r| r.clone()).collect()
    }

    /// Finds all sessions from a given IP (may be multiple for multi-accounts).
    pub fn find_by_ip(&self, ip: &IpAddr) -> Vec<SessionEntry> {
        self.sessions
            .iter()
            .filter(|r| r.client_ip == *ip)
            .map(|r| r.clone())
            .collect()
    }

    /// Finds the session with the given Mojang UUID.
    pub fn find_by_uuid(&self, uuid: &Uuid) -> Option<SessionEntry> {
        self.sessions
            .iter()
            .find(|r| r.player_uuid.as_ref() == Some(uuid))
            .map(|r| r.clone())
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(username: &str, server: &str) -> SessionEntry {
        SessionEntry {
            session_id: Uuid::new_v4(),
            username: Some(username.to_string()),
            player_uuid: None,
            client_ip: "127.0.0.1".parse().unwrap(),
            server_id: server.to_string(),
            proxy_mode: ProxyMode::Passthrough,
            connected_at: Instant::now(),
            shutdown_token: CancellationToken::new(),
        }
    }

    #[test]
    fn register_and_get() {
        let registry = ConnectionRegistry::new();
        let entry = make_entry("alice", "lobby");
        let id = registry.register(entry);
        let found = registry.get(&id).unwrap();
        assert_eq!(found.username.as_deref(), Some("alice"));
    }

    #[test]
    fn unregister_removes() {
        let registry = ConnectionRegistry::new();
        let entry = make_entry("bob", "survival");
        let id = registry.register(entry);
        assert!(registry.unregister(&id).is_some());
        assert!(registry.get(&id).is_none());
    }

    #[test]
    fn find_by_username() {
        let registry = ConnectionRegistry::new();
        registry.register(make_entry("alice", "lobby"));
        registry.register(make_entry("bob", "survival"));
        let found = registry.find_by_username("bob").unwrap();
        assert_eq!(found.server_id, "survival");
        assert!(registry.find_by_username("charlie").is_none());
    }

    #[test]
    fn count_by_server() {
        let registry = ConnectionRegistry::new();
        registry.register(make_entry("alice", "lobby"));
        registry.register(make_entry("bob", "lobby"));
        registry.register(make_entry("charlie", "survival"));
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
                let entry = make_entry(&format!("player_{i}"), "lobby");
                reg.register(entry);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(registry.count(), 10);
    }
}
