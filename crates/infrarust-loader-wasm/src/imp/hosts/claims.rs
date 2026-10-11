use crate::bindings::infrarust::plugin::connection_claims as wcc;
use crate::bindings::infrarust::plugin::types::{ErrorKind, HostError};
use crate::host_error::{HostResult, claim_error, host_error};
use crate::store_state::PluginStoreState;

fn no_registry() -> HostError {
    host_error(
        ErrorKind::Unsupported,
        "this proxy exposes no connection claim registry",
    )
}

impl wcc::Host for PluginStoreState {
    async fn register_prefix(&mut self, prefix: Vec<u8>) -> wasmtime::Result<HostResult<()>> {
        Ok(self.claim_prefix(&prefix))
    }

    async fn unregister_prefix(&mut self, prefix: Vec<u8>) -> wasmtime::Result<HostResult<()>> {
        Ok(self.release_prefix(&prefix))
    }
}

impl PluginStoreState {
    fn claim_prefix(&mut self, prefix: &[u8]) -> HostResult<()> {
        self.check(gate!("connection-claims", "register-prefix"))?;
        let ctx = self.services()?;
        let claims = ctx.connection_claims().ok_or_else(no_registry)?;
        claims
            .register_prefix(prefix)
            .map_err(|error| claim_error(&error))?;
        self.record_claim_prefix(prefix);
        Ok(())
    }

    fn release_prefix(&mut self, prefix: &[u8]) -> HostResult<()> {
        self.check(gate!("connection-claims", "unregister-prefix"))?;
        let ctx = self.services()?;
        let claims = ctx.connection_claims().ok_or_else(no_registry)?;
        claims
            .unregister_prefix(prefix)
            .map_err(|error| claim_error(&error))?;
        self.forget_claim_prefix(prefix);
        Ok(())
    }
}
