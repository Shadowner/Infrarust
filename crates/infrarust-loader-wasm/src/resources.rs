//! WIT resource backing for the host.

use std::sync::Arc;

use infrarust_api::limbo::LimboSession;
use infrarust_api::player::Player;
use wasmtime::component::Resource;

use crate::bindings::infrarust::plugin::limbo::LimboSession as LimboSessionHandle;
use crate::bindings::infrarust::plugin::player_registry::Player as PlayerHandle;
use crate::store_state::PluginStoreState;

pub(crate) struct PlayerResource {
    pub(crate) provider: Arc<dyn Player>,
}

/// Host-side storage for a `limbo-session` resource: the live native session.
pub(crate) struct LimboSessionResource {
    pub(crate) session: Arc<dyn LimboSession>,
}

impl PluginStoreState {
    pub(crate) fn push_player(
        &mut self,
        provider: Arc<dyn Player>,
    ) -> wasmtime::Result<Resource<PlayerHandle>> {
        let stored = self.table_mut().push(PlayerResource { provider })?;
        Ok(Resource::new_own(stored.rep()))
    }

    pub(crate) fn resolve_player(
        &mut self,
        handle: &Resource<PlayerHandle>,
    ) -> wasmtime::Result<Arc<dyn Player>> {
        let stored: &PlayerResource = self
            .table_mut()
            .get(&Resource::<PlayerResource>::new_own(handle.rep()))?;
        Ok(stored.provider.clone())
    }

    pub(crate) fn drop_player(&mut self, handle: Resource<PlayerHandle>) -> wasmtime::Result<()> {
        self.table_mut()
            .delete(Resource::<PlayerResource>::new_own(handle.rep()))?;
        Ok(())
    }

    pub(crate) fn push_limbo_session(
        &mut self,
        session: Arc<dyn LimboSession>,
    ) -> wasmtime::Result<Resource<LimboSessionHandle>> {
        let stored = self.table_mut().push(LimboSessionResource { session })?;
        Ok(Resource::new_own(stored.rep()))
    }

    pub(crate) fn resolve_limbo_session(
        &mut self,
        handle: &Resource<LimboSessionHandle>,
    ) -> wasmtime::Result<Arc<dyn LimboSession>> {
        let stored: &LimboSessionResource = self
            .table_mut()
            .get(&Resource::<LimboSessionResource>::new_own(handle.rep()))?;
        Ok(stored.session.clone())
    }

    pub(crate) fn drop_limbo_session(
        &mut self,
        handle: Resource<LimboSessionHandle>,
    ) -> wasmtime::Result<()> {
        self.table_mut()
            .delete(Resource::<LimboSessionResource>::new_own(handle.rep()))?;
        Ok(())
    }
}
