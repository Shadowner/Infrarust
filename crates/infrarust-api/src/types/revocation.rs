use std::sync::Arc;

#[derive(Clone)]
pub struct Revocation(Arc<dyn Fn() -> bool + Send + Sync>);

impl Revocation {
    pub fn new(revoke: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(revoke))
    }

    pub fn revoke(&self) -> bool {
        (self.0)()
    }
}

impl std::fmt::Debug for Revocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Revocation").finish_non_exhaustive()
    }
}
