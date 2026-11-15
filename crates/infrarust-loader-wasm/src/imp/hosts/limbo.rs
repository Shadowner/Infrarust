use std::sync::Arc;

use infrarust_api::limbo::{LimboOutcome, LimboSession, SessionHandle};
use wasmtime::component::Resource;

use super::parse_text;
use crate::actor::CallKind;
use crate::adapters::WasmLimboHandler;
use crate::bindings::infrarust::plugin::limbo as wl;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::host_error::{HostResult, invalid_component, limbo_error, player_error};
use crate::registrations::Bound;
use crate::store_state::{PluginStoreState, Quota};

impl wl::Host for PluginStoreState {
    async fn register_limbo_handler(
        &mut self,
        name: String,
        handler: u64,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.register_limbo(name, handler))
    }
}

impl PluginStoreState {
    fn register_limbo(&mut self, name: String, handler: u64) -> HostResult<()> {
        self.check(gate!("limbo", "register-limbo-handler"))?;
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Callback)?.any_generation();
        let limit = self.quota(Quota::LimboHandlers);
        let binding =
            match self
                .registrations()
                .limbo()
                .bind(&name, self.generation(), handler, limit)
            {
                Bound::Fresh(binding) => binding,
                Bound::Rebound => return Ok(()),
                Bound::Full => return Err(self.quota_exceeded(Quota::LimboHandlers)),
            };
        let registration = ctx
            .register_limbo_handler(Box::new(WasmLimboHandler::new(
                binding,
                name.clone(),
                instance,
                Arc::clone(self.registrations()),
            )))
            .map_err(|error| {
                self.registrations().limbo().unbind(&name);
                tracing::warn!(plugin = %self.plugin_id(), %error, "limbo handler refused");
                limbo_error(&error)
            })?;
        self.registrations().limbo().record(&name, registration);
        Ok(())
    }
}

fn message(session: &dyn LimboSession, message: &wt::Component) -> HostResult<()> {
    let message = parse_text(message)?;
    session.send_message(message).map_err(player_error)
}

fn title(session: &dyn LimboSession, title: &wt::TitleData) -> HostResult<()> {
    let title = convert::title_data_from_wit(title).map_err(|e| invalid_component(&e))?;
    session.send_title(title).map_err(player_error)
}

fn action_bar(session: &dyn LimboSession, message: &wt::Component) -> HostResult<()> {
    let message = parse_text(message)?;
    session.send_action_bar(message).map_err(player_error)
}

fn complete(outcome: &wl::HandlerResult, complete: impl FnOnce(LimboOutcome)) -> HostResult<()> {
    complete(convert::complete_result_from_wit(outcome)?);
    Ok(())
}

impl wl::HostLimboSession for PluginStoreState {
    async fn player_id(&mut self, self_: Resource<wl::LimboSession>) -> wasmtime::Result<u64> {
        Ok(self.resolve_limbo_session(&self_)?.player_id().as_u64())
    }

    async fn profile(
        &mut self,
        self_: Resource<wl::LimboSession>,
    ) -> wasmtime::Result<wt::GameProfile> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(convert::game_profile_to_wit(session.profile()))
    }

    async fn entry_context(
        &mut self,
        self_: Resource<wl::LimboSession>,
    ) -> wasmtime::Result<wl::LimboEntryContext> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(convert::limbo_entry_context_to_wit(session.entry_context()))
    }

    async fn send_message(
        &mut self,
        self_: Resource<wl::LimboSession>,
        text: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(message(&*self.resolve_limbo_session(&self_)?, &text))
    }

    async fn send_title(
        &mut self,
        self_: Resource<wl::LimboSession>,
        data: wt::TitleData,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(title(&*self.resolve_limbo_session(&self_)?, &data))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<wl::LimboSession>,
        text: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(action_bar(&*self.resolve_limbo_session(&self_)?, &text))
    }

    async fn complete(
        &mut self,
        self_: Resource<wl::LimboSession>,
        outcome: wl::HandlerResult,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(complete(&outcome, |outcome| session.complete(outcome)))
    }

    async fn acquire_handle(
        &mut self,
        self_: Resource<wl::LimboSession>,
    ) -> wasmtime::Result<Resource<wl::LimboSessionHandle>> {
        let session = self.resolve_limbo_session(&self_)?;
        self.push_limbo_session_handle(session.handle())
    }

    async fn drop(&mut self, rep: Resource<wl::LimboSession>) -> wasmtime::Result<()> {
        let _ = self.drop_limbo_session(rep);
        Ok(())
    }
}

impl wl::HostLimboSessionHandle for PluginStoreState {
    async fn player_id(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
    ) -> wasmtime::Result<u64> {
        Ok(self
            .resolve_limbo_session_handle(&self_)?
            .player_id()
            .as_u64())
    }

    async fn send_message(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        text: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session_handle(&self_)?.as_session();
        Ok(message(&*session, &text))
    }

    async fn send_title(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        data: wt::TitleData,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session_handle(&self_)?.as_session();
        Ok(title(&*session, &data))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        text: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session_handle(&self_)?.as_session();
        Ok(action_bar(&*session, &text))
    }

    async fn complete(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        outcome: wl::HandlerResult,
    ) -> wasmtime::Result<HostResult<()>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(complete(&outcome, |outcome| handle.complete(outcome)))
    }

    async fn cancelled(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
    ) -> wasmtime::Result<bool> {
        Ok(self
            .resolve_limbo_session_handle(&self_)?
            .cancellation_token()
            .is_cancelled())
    }

    async fn drop(&mut self, rep: Resource<wl::LimboSessionHandle>) -> wasmtime::Result<()> {
        let _ = self.drop_limbo_session_handle(rep);
        Ok(())
    }
}

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
