use std::future::Future;
use std::sync::Arc;

use infrarust_api::__private::session_task;
use infrarust_api::types::PlayerId;

pub(crate) const MAX_ENTRIES: u32 = 8;

tokio::task_local! {
    static CHAIN: CallChain;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Caller {
    plugin: Arc<str>,
    entries: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CallChain {
    callers: Arc<[Caller]>,
    session: Option<PlayerId>,
}

impl CallChain {
    pub(crate) fn current() -> Self {
        let callers = CHAIN
            .try_with(|chain| Arc::clone(&chain.callers))
            .unwrap_or_default();
        Self {
            callers,
            session: session_task::current(),
        }
    }

    pub(crate) fn contains(&self, plugin_id: &str) -> bool {
        self.entries(plugin_id) > 0
    }

    pub(crate) fn entries(&self, plugin_id: &str) -> u32 {
        self.callers
            .iter()
            .find(|caller| &*caller.plugin == plugin_id)
            .map_or(0, |caller| caller.entries)
    }

    pub(crate) fn with(&self, plugin_id: &str) -> Self {
        let mut callers: Vec<Caller> = self.callers.to_vec();
        match callers
            .iter_mut()
            .find(|caller| &*caller.plugin == plugin_id)
        {
            Some(caller) => caller.entries = caller.entries.saturating_add(1),
            None => callers.push(Caller {
                plugin: Arc::from(plugin_id),
                entries: 1,
            }),
        }
        Self {
            callers: callers.into(),
            session: self.session,
        }
    }

    pub(crate) fn unawaited(self) -> Self {
        Self {
            session: None,
            ..self
        }
    }

    pub(crate) async fn scope<F: Future>(self, running: F) -> F::Output {
        let session = self.session;
        CHAIN
            .scope(self, session_task::scope(session, running))
            .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn a_scope_is_seen_by_the_code_it_runs_and_nested_scopes_accumulate() {
        assert!(!CallChain::current().contains("a"));
        let outer = CallChain::default().with("a");
        let seen = outer
            .clone()
            .scope(async {
                let inner = CallChain::current().with("b");
                let nested = inner
                    .scope(async { (CallChain::current().contains("a"), CallChain::current()) })
                    .await;
                (CallChain::current(), nested)
            })
            .await;
        assert_eq!(seen.0, outer);
        assert!(seen.1.0, "the outer caller is still in the chain");
        assert!(seen.1.1.contains("b"));
        assert!(!CallChain::current().contains("a"), "the scope ended");
    }

    #[test]
    fn a_caller_is_recorded_once_with_the_number_of_times_the_chain_entered_it() {
        let chain = CallChain::default().with("a").with("b").with("a");
        assert_eq!(chain.entries("a"), 2);
        assert_eq!(chain.entries("b"), 1);
        assert_eq!(chain.entries("c"), 0);
        assert!(!chain.contains("c"));
        assert_eq!(chain.callers.len(), 2, "a plugin is listed once: {chain:?}");
        let mut relay = CallChain::default();
        for _ in 0..MAX_ENTRIES {
            relay = relay.with("a").with("b");
        }
        assert_eq!(relay.entries("a"), MAX_ENTRIES);
        assert_eq!(relay.callers.len(), 2);
    }

    #[tokio::test]
    async fn the_waiting_session_rides_along_with_the_chain_and_is_restored_in_its_scope() {
        let steve = PlayerId::new(7);
        let captured =
            session_task::scope(Some(steve), async { CallChain::current().with("a") }).await;
        assert_eq!(session_task::current(), None);
        let seen = captured
            .clone()
            .scope(async { (session_task::current(), CallChain::current()) })
            .await;
        assert_eq!(seen, (Some(steve), captured.clone()));
        assert_eq!(captured.with("b").session, Some(steve));
        let posted = captured.unawaited();
        assert!(posted.contains("a"));
        assert_eq!(
            posted.scope(async { session_task::current() }).await,
            None,
            "a job nobody waits for does not carry the session"
        );
    }

    #[tokio::test]
    async fn a_spawned_task_starts_with_an_empty_chain() {
        let spawned = CallChain::default()
            .with("a")
            .scope(async { tokio::spawn(async { CallChain::current() }).await.unwrap() })
            .await;
        assert_eq!(spawned, CallChain::default());
    }
}
