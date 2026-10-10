use std::sync::Arc;

use infrarust_api::limbo::{LimboSession, SessionHandle};
use wasmtime::component::Resource;

use crate::bindings::infrarust::plugin::limbo::{
    LimboSession as LimboSessionMarker, LimboSessionHandle as LimboHandleMarker,
};
use crate::store_state::PluginStoreState;

pub(crate) struct LimboSessionResource {
    pub(crate) session: Arc<dyn LimboSession>,
}

pub(crate) struct LimboSessionHandleResource {
    pub(crate) handle: SessionHandle,
}

impl PluginStoreState {
    pub(crate) fn push_limbo_session(
        &mut self,
        session: Arc<dyn LimboSession>,
    ) -> wasmtime::Result<Resource<LimboSessionMarker>> {
        let stored = self.table_mut().push(LimboSessionResource { session })?;
        Ok(Resource::new_own(stored.rep()))
    }

    pub(crate) fn resolve_limbo_session(
        &mut self,
        handle: &Resource<LimboSessionMarker>,
    ) -> wasmtime::Result<Arc<dyn LimboSession>> {
        let stored: &LimboSessionResource =
            self.table_mut()
                .get(&Resource::<LimboSessionResource>::new_own(handle.rep()))?;
        Ok(stored.session.clone())
    }

    pub(crate) fn drop_limbo_session(
        &mut self,
        handle: Resource<LimboSessionMarker>,
    ) -> wasmtime::Result<()> {
        self.table_mut()
            .delete(Resource::<LimboSessionResource>::new_own(handle.rep()))?;
        Ok(())
    }

    pub(crate) fn push_limbo_session_handle(
        &mut self,
        handle: SessionHandle,
    ) -> wasmtime::Result<Resource<LimboHandleMarker>> {
        let stored = self
            .table_mut()
            .push(LimboSessionHandleResource { handle })?;
        Ok(Resource::new_own(stored.rep()))
    }

    pub(crate) fn resolve_limbo_session_handle(
        &mut self,
        handle: &Resource<LimboHandleMarker>,
    ) -> wasmtime::Result<SessionHandle> {
        let stored: &LimboSessionHandleResource = self.table_mut().get(&Resource::<
            LimboSessionHandleResource,
        >::new_own(
            handle.rep()
        ))?;
        Ok(stored.handle.clone())
    }

    pub(crate) fn drop_limbo_session_handle(
        &mut self,
        handle: Resource<LimboHandleMarker>,
    ) -> wasmtime::Result<()> {
        let _ = self
            .table_mut()
            .delete(Resource::<LimboSessionHandleResource>::new_own(
                handle.rep(),
            ));
        Ok(())
    }
}
