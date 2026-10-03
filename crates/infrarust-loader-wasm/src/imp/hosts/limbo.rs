use std::sync::Arc;

use infrarust_api::error::PlayerError;
use infrarust_api::limbo::{LimboOutcome, LimboSession, SessionHandle};
use infrarust_api::types::{Component, PlayerId, TitleData};
use wasmtime::component::Resource;

use super::parse_text;
use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::limbo as wl;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::host_error::{HostResult, invalid_component, limbo_error, player_error};
use crate::limbo::WasmLimboHandler;
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
                .bind_limbo(&name, self.generation(), handler, limit)
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
                self.registrations().unbind_limbo(&name);
                tracing::warn!(plugin = %self.plugin_id(), %error, "limbo handler refused");
                limbo_error(&error)
            })?;
        self.registrations()
            .record_limbo_registration(&name, registration);
        Ok(())
    }
}

trait LimboTarget {
    fn player_id(&self) -> PlayerId;
    fn send_message(&self, message: Component) -> Result<(), PlayerError>;
    fn send_title(&self, title: TitleData) -> Result<(), PlayerError>;
    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError>;
    fn complete(&self, outcome: LimboOutcome);
}

impl LimboTarget for Arc<dyn LimboSession> {
    fn player_id(&self) -> PlayerId {
        LimboSession::player_id(&**self)
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        LimboSession::send_message(&**self, message)
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        LimboSession::send_title(&**self, title)
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        LimboSession::send_action_bar(&**self, message)
    }

    fn complete(&self, outcome: LimboOutcome) {
        LimboSession::complete(&**self, outcome);
    }
}

impl LimboTarget for SessionHandle {
    fn player_id(&self) -> PlayerId {
        Self::player_id(self)
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        Self::send_message(self, message)
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        Self::send_title(self, title)
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        Self::send_action_bar(self, message)
    }

    fn complete(&self, outcome: LimboOutcome) {
        Self::complete(self, outcome);
    }
}

fn player_id(target: &impl LimboTarget) -> u64 {
    target.player_id().as_u64()
}

fn message(target: &impl LimboTarget, message: &wt::Component) -> HostResult<()> {
    let message = parse_text(message)?;
    target.send_message(message).map_err(player_error)
}

fn title(target: &impl LimboTarget, title: &wt::TitleData) -> HostResult<()> {
    let title = convert::title_data_from_wit(title).map_err(|e| invalid_component(&e))?;
    target.send_title(title).map_err(player_error)
}

fn action_bar(target: &impl LimboTarget, message: &wt::Component) -> HostResult<()> {
    let message = parse_text(message)?;
    target.send_action_bar(message).map_err(player_error)
}

fn complete(target: &impl LimboTarget, outcome: &wl::HandlerResult) -> HostResult<()> {
    let outcome = convert::complete_result_from_wit(outcome)?;
    target.complete(outcome);
    Ok(())
}

impl wl::HostLimboSession for PluginStoreState {
    async fn player_id(&mut self, self_: Resource<wl::LimboSession>) -> wasmtime::Result<u64> {
        Ok(player_id(&self.resolve_limbo_session(&self_)?))
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
        Ok(message(&self.resolve_limbo_session(&self_)?, &text))
    }

    async fn send_title(
        &mut self,
        self_: Resource<wl::LimboSession>,
        data: wt::TitleData,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(title(&self.resolve_limbo_session(&self_)?, &data))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<wl::LimboSession>,
        text: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(action_bar(&self.resolve_limbo_session(&self_)?, &text))
    }

    async fn complete(
        &mut self,
        self_: Resource<wl::LimboSession>,
        outcome: wl::HandlerResult,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(complete(&self.resolve_limbo_session(&self_)?, &outcome))
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
        Ok(player_id(&self.resolve_limbo_session_handle(&self_)?))
    }

    async fn send_message(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        text: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(message(&self.resolve_limbo_session_handle(&self_)?, &text))
    }

    async fn send_title(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        data: wt::TitleData,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(title(&self.resolve_limbo_session_handle(&self_)?, &data))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        text: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(action_bar(
            &self.resolve_limbo_session_handle(&self_)?,
            &text,
        ))
    }

    async fn complete(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        outcome: wl::HandlerResult,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(complete(
            &self.resolve_limbo_session_handle(&self_)?,
            &outcome,
        ))
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
