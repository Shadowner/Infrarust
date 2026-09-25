use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::handle::SessionHandle;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler, SessionEndReason};
use infrarust_api::limbo::registration::LimboHandlerError;
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::types::PlayerId;

use crate::error::CoreError;

#[derive(Default)]
struct Holds {
    removed: bool,
    sessions: HashMap<PlayerId, SessionHandle>,
}

struct ManagedHandler {
    inner: Box<dyn LimboHandler>,
    holds: Mutex<Holds>,
}

impl ManagedHandler {
    fn holds(&self) -> MutexGuard<'_, Holds> {
        self.holds.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_removed(&self) -> bool {
        self.holds().removed
    }

    fn forget(&self, player: PlayerId) -> bool {
        let mut holds = self.holds();
        holds.sessions.remove(&player);
        holds.removed
    }

    fn release(&self) -> usize {
        let held = {
            let mut holds = self.holds();
            holds.removed = true;
            std::mem::take(&mut holds.sessions)
        };
        let live: Vec<&SessionHandle> = held.values().filter(|h| !ended(h)).collect();
        for handle in &live {
            handle.complete(HandlerResult::unavailable());
        }
        live.len()
    }
}

impl LimboHandler for ManagedHandler {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async move {
            if self.is_removed() {
                return HandlerResult::unavailable();
            }
            let result = self.inner.on_player_enter(session).await;
            if matches!(
                result,
                HandlerResult::Hold | HandlerResult::HoldWithTimeout { .. }
            ) {
                let mut holds = self.holds();
                if holds.removed {
                    return HandlerResult::unavailable();
                }
                holds.sessions.retain(|_, handle| !ended(handle));
                holds.sessions.insert(session.player_id(), session.handle());
            }
            result
        })
    }

    fn on_command<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        if self.is_removed() {
            return Box::pin(async {});
        }
        self.inner.on_command(session, command, args)
    }

    fn on_chat<'a>(&'a self, session: &'a dyn LimboSession, message: &'a str) -> BoxFuture<'a, ()> {
        if self.is_removed() {
            return Box::pin(async {});
        }
        self.inner.on_chat(session, message)
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        if self.forget(player_id) {
            return Box::pin(async {});
        }
        self.inner.on_disconnect(player_id)
    }

    fn on_session_end(&self, player_id: PlayerId, reason: SessionEndReason) -> BoxFuture<'_, ()> {
        if self.forget(player_id) {
            return Box::pin(async {});
        }
        self.inner.on_session_end(player_id, reason)
    }
}

fn ended(handle: &SessionHandle) -> bool {
    handle.cancellation_token().is_cancelled()
}

#[derive(Clone)]
struct Entry {
    id: u64,
    owner: Arc<str>,
    handler: Arc<ManagedHandler>,
}

pub struct LimboHandlerRegistry {
    entries: RwLock<HashMap<String, Entry>>,
    next_id: AtomicU64,
}

impl LimboHandlerRegistry {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn register(
        &self,
        owner: &str,
        handler: Box<dyn LimboHandler>,
    ) -> Result<u64, LimboHandlerError> {
        let name = handler.name().to_string();
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = entries.get(&name) {
            return Err(LimboHandlerError::NameTaken {
                name,
                owner: existing.owner.to_string(),
            });
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        entries.insert(
            name,
            Entry {
                id,
                owner: Arc::from(owner),
                handler: Arc::new(ManagedHandler {
                    inner: handler,
                    holds: Mutex::new(Holds::default()),
                }),
            },
        );
        Ok(id)
    }

    pub fn unregister(&self, id: u64) -> bool {
        let removed = {
            let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
            let name = entries
                .iter()
                .find(|(_, entry)| entry.id == id)
                .map(|(name, _)| name.clone());
            name.and_then(|name| entries.remove(&name))
        };
        match removed {
            Some(entry) => {
                release(&entry);
                true
            }
            None => false,
        }
    }

    pub fn unregister_owner(&self, owner: &str) -> usize {
        let removed: Vec<Entry> = {
            let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
            let names: Vec<String> = entries
                .iter()
                .filter(|(_, entry)| &*entry.owner == owner)
                .map(|(name, _)| name.clone())
                .collect();
            names
                .into_iter()
                .filter_map(|name| entries.remove(&name))
                .collect()
        };
        for entry in &removed {
            release(entry);
        }
        removed.len()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn LimboHandler>> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .map(|entry| Arc::clone(&entry.handler) as Arc<dyn LimboHandler>)
    }

    pub fn owner_of(&self, name: &str) -> Option<String> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .map(|entry| entry.owner.to_string())
    }

    pub fn owned_by(&self, owner: &str) -> Vec<Arc<dyn LimboHandler>> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|entry| &*entry.owner == owner)
            .map(|entry| Arc::clone(&entry.handler) as Arc<dyn LimboHandler>)
            .collect()
    }

    pub fn resolve_handlers(
        &self,
        names: &[String],
    ) -> Result<Vec<Arc<dyn LimboHandler>>, CoreError> {
        names
            .iter()
            .map(|name| {
                self.get(name)
                    .ok_or_else(|| CoreError::Other(format!("limbo handler not found: {name}")))
            })
            .collect()
    }

    pub fn resolve_handlers_lenient(&self, names: &[String]) -> Vec<Arc<dyn LimboHandler>> {
        names
            .iter()
            .filter_map(|name| match self.get(name) {
                Some(h) => Some(h),
                None => {
                    tracing::warn!(handler = %name, "limbo handler not found, skipping");
                    None
                }
            })
            .collect()
    }
}

fn release(entry: &Entry) {
    let released = entry.handler.release();
    tracing::debug!(
        plugin = %entry.owner,
        handler = %entry.handler.name(),
        released,
        "limbo handler removed"
    );
}

impl Default for LimboHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::AtomicUsize;

    use infrarust_api::limbo::context::LimboEntryContext;
    use infrarust_api::limbo::test_util::RecordingLimboSession;
    use infrarust_api::types::{GameProfile, ServerId};

    use super::*;

    struct StubHandler {
        handler_name: &'static str,
        result: HandlerResult,
        calls: Arc<AtomicUsize>,
    }

    impl StubHandler {
        fn new(handler_name: &'static str) -> Box<Self> {
            Self::holding(handler_name, HandlerResult::Accept)
        }

        fn holding(handler_name: &'static str, result: HandlerResult) -> Box<Self> {
            Box::new(Self {
                handler_name,
                result,
                calls: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    impl LimboHandler for StubHandler {
        fn name(&self) -> &str {
            self.handler_name
        }

        fn on_player_enter<'a>(
            &'a self,
            _session: &'a dyn LimboSession,
        ) -> BoxFuture<'a, HandlerResult> {
            let result = self.result.clone();
            Box::pin(async move { result })
        }

        fn on_session_end(
            &self,
            _player: PlayerId,
            _reason: SessionEndReason,
        ) -> BoxFuture<'_, ()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }
    }

    fn session(player: u64) -> Arc<RecordingLimboSession> {
        RecordingLimboSession::new(
            PlayerId::new(player),
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: format!("p{player}"),
                properties: vec![],
            },
            LimboEntryContext::InitialConnection {
                target_server: ServerId::from("hub"),
            },
        )
    }

    fn is_unavailable(results: &[HandlerResult]) -> bool {
        matches!(results, [HandlerResult::Deny(reason)] if reason.to_plain() == infrarust_api::limbo::HANDLER_UNAVAILABLE)
    }

    #[test]
    fn register_and_get() {
        let registry = LimboHandlerRegistry::new();
        registry.register("p", StubHandler::new("auth")).unwrap();
        assert_eq!(registry.get("auth").unwrap().name(), "auth");
        assert_eq!(registry.owner_of("auth").as_deref(), Some("p"));
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn a_taken_name_is_refused_and_the_first_registration_kept() {
        let registry = LimboHandlerRegistry::new();
        registry
            .register("first", StubHandler::new("auth"))
            .unwrap();
        assert_eq!(
            registry.register("second", StubHandler::new("auth")),
            Err(LimboHandlerError::NameTaken {
                name: "auth".into(),
                owner: "first".into(),
            })
        );
        assert!(
            registry
                .register("first", StubHandler::new("auth"))
                .is_err()
        );
        assert_eq!(registry.owner_of("auth").as_deref(), Some("first"));
    }

    #[test]
    fn unregister_by_id_only_removes_that_registration() {
        let registry = LimboHandlerRegistry::new();
        let id = registry.register("p", StubHandler::new("auth")).unwrap();
        assert!(registry.unregister(id));
        assert!(!registry.unregister(id));
        assert!(registry.get("auth").is_none());
        let again = registry.register("q", StubHandler::new("auth")).unwrap();
        assert!(!registry.unregister(id));
        assert_ne!(again, id);
        assert!(registry.get("auth").is_some());
    }

    #[test]
    fn unregister_owner_removes_every_handler_of_that_plugin() {
        let registry = LimboHandlerRegistry::new();
        registry.register("p", StubHandler::new("a")).unwrap();
        registry.register("p", StubHandler::new("b")).unwrap();
        registry.register("q", StubHandler::new("c")).unwrap();
        assert_eq!(registry.owned_by("p").len(), 2);
        assert_eq!(registry.unregister_owner("p"), 2);
        assert!(registry.get("a").is_none() && registry.get("b").is_none());
        assert!(registry.get("c").is_some());
    }

    #[tokio::test]
    async fn removing_a_handler_releases_the_players_it_holds() {
        let registry = LimboHandlerRegistry::new();
        let stub = StubHandler::holding("gate", HandlerResult::Hold);
        let session_ends = Arc::clone(&stub.calls);
        registry.register("p", stub).unwrap();
        let gate = registry.get("gate").unwrap();
        let (held, left) = (session(1), session(2));
        assert!(matches!(
            gate.on_player_enter(held.as_ref()).await,
            HandlerResult::Hold
        ));
        gate.on_player_enter(left.as_ref()).await;
        gate.on_session_end(PlayerId::new(2), SessionEndReason::Released)
            .await;

        assert_eq!(registry.unregister_owner("p"), 1);

        assert!(is_unavailable(&held.completions()));
        assert!(left.completions().is_empty());
        let late = session(3);
        assert!(is_unavailable(&[gate.on_player_enter(late.as_ref()).await]));
        gate.on_session_end(PlayerId::new(1), SessionEndReason::Kicked)
            .await;
        assert_eq!(session_ends.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_hold_whose_session_ended_without_notice_is_not_kept() {
        let registry = LimboHandlerRegistry::new();
        registry
            .register("p", StubHandler::holding("gate", HandlerResult::Hold))
            .unwrap();
        let gate = registry.get("gate").unwrap();
        let (moved_on, held) = (session(1), session(2));
        gate.on_player_enter(moved_on.as_ref()).await;
        moved_on.cancellation_token().cancel();
        gate.on_player_enter(held.as_ref()).await;

        registry.unregister_owner("p");

        assert!(moved_on.completions().is_empty());
        assert!(is_unavailable(&held.completions()));
    }

    #[test]
    fn resolve_handlers_all_present() {
        let registry = LimboHandlerRegistry::new();
        registry.register("p", StubHandler::new("auth")).unwrap();
        registry.register("p", StubHandler::new("lobby")).unwrap();

        let names = vec!["auth".to_string(), "lobby".to_string()];
        let resolved = registry.resolve_handlers(&names).unwrap();

        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].name(), "auth");
        assert_eq!(resolved[1].name(), "lobby");
    }

    #[test]
    fn resolve_handlers_missing_returns_error() {
        let registry = LimboHandlerRegistry::new();
        registry.register("p", StubHandler::new("auth")).unwrap();

        let names = vec!["auth".to_string(), "missing".to_string()];
        let err = registry.resolve_handlers(&names).err().unwrap();
        assert!(err.to_string().contains("missing"), "{err}");
        assert_eq!(registry.resolve_handlers_lenient(&names).len(), 1);
    }

    #[test]
    fn resolve_empty_list() {
        let registry = LimboHandlerRegistry::new();
        assert!(registry.resolve_handlers(&[]).unwrap().is_empty());
    }
}
