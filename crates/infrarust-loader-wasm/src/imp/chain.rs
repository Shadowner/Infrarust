use std::future::Future;
use std::sync::Arc;

tokio::task_local! {
    static CHAIN: CallChain;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CallChain(Arc<[Arc<str>]>);

impl CallChain {
    pub(crate) fn current() -> Self {
        CHAIN.try_with(Clone::clone).unwrap_or_default()
    }

    pub(crate) fn contains(&self, plugin_id: &str) -> bool {
        self.0.iter().any(|caller| &**caller == plugin_id)
    }

    pub(crate) fn with(&self, plugin_id: &str) -> Self {
        if self.contains(plugin_id) {
            return self.clone();
        }
        let mut callers: Vec<Arc<str>> = self.0.to_vec();
        callers.push(Arc::from(plugin_id));
        Self(callers.into())
    }

    pub(crate) async fn scope<F: Future>(self, running: F) -> F::Output {
        CHAIN.scope(self, running).await
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(outer.with("a"), outer, "a caller is recorded once");
        assert!(!CallChain::current().contains("a"), "the scope ended");
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
