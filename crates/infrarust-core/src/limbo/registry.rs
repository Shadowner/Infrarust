//! [`LimboHandlerRegistry`] — maps handler names to handler instances.
//!
//! Provides thread-safe registration and lookup of [`LimboHandler`] instances
//! by name. Used by the limbo engine to resolve handler chains from config.

use std::sync::Arc;

use dashmap::DashMap;

use infrarust_api::limbo::handler::LimboHandler;

use crate::error::CoreError;

/// Thread-safe registry mapping handler names to [`LimboHandler`] instances.
///
/// Uses [`DashMap`] for concurrent read/write access without external locking.
pub struct LimboHandlerRegistry {
    handlers: DashMap<String, Arc<dyn LimboHandler>>,
}

impl LimboHandlerRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
        }
    }

    /// Registers a handler, keyed by its [`LimboHandler::name()`].
    ///
    /// Overwrites any existing handler with the same name.
    pub fn register(&self, handler: Arc<dyn LimboHandler>) {
        self.handlers.insert(handler.name().to_string(), handler);
    }

    /// Returns the handler registered under `name`, if any.
    pub fn get(&self, name: &str) -> Option<Arc<dyn LimboHandler>> {
        self.handlers.get(name).map(|entry| Arc::clone(entry.value()))
    }

    /// Resolves an ordered list of handler names into handler instances.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Other`] if any name has no registered handler.
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
}

impl Default for LimboHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use infrarust_api::event::BoxFuture;
    use infrarust_api::limbo::handler::HandlerResult;
    use infrarust_api::limbo::session::LimboSession;

    struct StubHandler {
        handler_name: &'static str,
    }

    impl LimboHandler for StubHandler {
        fn name(&self) -> &str {
            self.handler_name
        }

        fn on_player_enter(&self, _session: &dyn LimboSession) -> BoxFuture<'_, HandlerResult> {
            Box::pin(async { HandlerResult::Accept })
        }
    }

    #[test]
    fn register_and_get() {
        let registry = LimboHandlerRegistry::new();
        let handler: Arc<dyn LimboHandler> = Arc::new(StubHandler {
            handler_name: "auth",
        });

        registry.register(handler);

        let retrieved = registry.get("auth");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name(), "auth");
    }

    #[test]
    fn get_missing_returns_none() {
        let registry = LimboHandlerRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn register_overwrites_existing() {
        let registry = LimboHandlerRegistry::new();

        let h1: Arc<dyn LimboHandler> = Arc::new(StubHandler {
            handler_name: "auth",
        });
        let h2: Arc<dyn LimboHandler> = Arc::new(StubHandler {
            handler_name: "auth",
        });

        registry.register(h1);
        registry.register(h2);

        // Should still resolve fine — the second registration wins.
        assert!(registry.get("auth").is_some());
    }

    #[test]
    fn resolve_handlers_all_present() {
        let registry = LimboHandlerRegistry::new();

        registry.register(Arc::new(StubHandler {
            handler_name: "auth",
        }));
        registry.register(Arc::new(StubHandler {
            handler_name: "lobby",
        }));

        let names = vec!["auth".to_string(), "lobby".to_string()];
        let resolved = registry.resolve_handlers(&names).unwrap();

        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].name(), "auth");
        assert_eq!(resolved[1].name(), "lobby");
    }

    #[test]
    fn resolve_handlers_missing_returns_error() {
        let registry = LimboHandlerRegistry::new();

        registry.register(Arc::new(StubHandler {
            handler_name: "auth",
        }));

        let names = vec!["auth".to_string(), "missing".to_string()];
        let result = registry.resolve_handlers(&names);

        assert!(result.is_err());
        match result {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(
                    err_msg.contains("missing"),
                    "error should name the missing handler: {err_msg}"
                );
            }
            Ok(_) => panic!("expected error for missing handler"),
        }
    }

    #[test]
    fn resolve_empty_list() {
        let registry = LimboHandlerRegistry::new();
        let resolved = registry.resolve_handlers(&[]).unwrap();
        assert!(resolved.is_empty());
    }
}
