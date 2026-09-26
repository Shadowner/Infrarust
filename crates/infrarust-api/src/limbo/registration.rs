use crate::types::Revocation;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LimboHandlerError {
    #[error("the plugin lacks the limbo capability")]
    MissingCapability,
    #[error("limbo handler `{name}` is already registered by `{owner}`")]
    NameTaken { name: String, owner: String },
}

#[derive(Debug, Clone)]
pub struct LimboHandlerRegistration {
    name: String,
    revocation: Revocation,
}

impl LimboHandlerRegistration {
    pub fn new(name: impl Into<String>, revoke: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            name: name.into(),
            revocation: Revocation::new(revoke),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn unregister(&self) -> bool {
        self.revocation.revoke()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn unregister_runs_the_revoke_once_per_call() {
        let live = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&live);
        let registration =
            LimboHandlerRegistration::new("gate", move || flag.swap(false, Ordering::SeqCst));
        assert_eq!(registration.name(), "gate");
        assert!(registration.clone().unregister());
        assert!(!registration.unregister());
    }
}
