use std::sync::Arc;

use infrarust_api::limbo::{LimboSession, SessionHandle};
use wasmtime::component::Resource;

use crate::store_state::PluginStoreState;

pub struct LimboSessionResource {
    session: Arc<dyn LimboSession>,
}

pub struct LimboSessionHandleResource {
    handle: SessionHandle,
}

impl PluginStoreState {
    pub(crate) fn push_limbo_session(
        &mut self,
        session: Arc<dyn LimboSession>,
    ) -> wasmtime::Result<Resource<LimboSessionResource>> {
        Ok(self.table_mut().push(LimboSessionResource { session })?)
    }

    pub(crate) fn resolve_limbo_session(
        &mut self,
        lent: &Resource<LimboSessionResource>,
    ) -> wasmtime::Result<Arc<dyn LimboSession>> {
        Ok(Arc::clone(&self.table_mut().get(lent)?.session))
    }

    pub(crate) fn drop_limbo_session(
        &mut self,
        lent: Resource<LimboSessionResource>,
    ) -> wasmtime::Result<()> {
        self.table_mut().delete(lent)?;
        Ok(())
    }

    pub(crate) fn push_limbo_session_handle(
        &mut self,
        handle: SessionHandle,
    ) -> wasmtime::Result<Resource<LimboSessionHandleResource>> {
        Ok(self
            .table_mut()
            .push(LimboSessionHandleResource { handle })?)
    }

    pub(crate) fn resolve_limbo_session_handle(
        &mut self,
        held: &Resource<LimboSessionHandleResource>,
    ) -> wasmtime::Result<SessionHandle> {
        Ok(self.table_mut().get(held)?.handle.clone())
    }

    pub(crate) fn drop_limbo_session_handle(
        &mut self,
        held: Resource<LimboSessionHandleResource>,
    ) -> wasmtime::Result<()> {
        self.table_mut().delete(held)?;
        Ok(())
    }
}
