use std::sync::Arc;

use infrarust_api::permissions::Capability;
use wasmtime::component::Resource;

use super::parse_text;
use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::limbo as wl;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::host_error::{HostResult, invalid_component, limbo_error, player_error};
use crate::limbo::WasmLimboHandler;
use crate::registrations::Bound;
use crate::store_state::PluginStoreState;

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
        self.check(Capability::Limbo, "limbo.register-limbo-handler")?;
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Callback).any_generation();
        let Bound::Fresh(binding) =
            self.registrations()
                .bind_limbo(&name, self.generation(), handler)
        else {
            return Ok(());
        };
        ctx.register_limbo_handler(Box::new(WasmLimboHandler::new(
            binding,
            name,
            instance,
            Arc::clone(self.registrations()),
        )))
        .map(|_| ())
        .map_err(|error| {
            tracing::warn!(plugin = %self.plugin_id, %error, "limbo handler refused");
            limbo_error(&error)
        })
    }
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
        message: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(parse_text(&message)
            .and_then(|message| session.send_message(message).map_err(player_error)))
    }

    async fn send_title(
        &mut self,
        self_: Resource<wl::LimboSession>,
        title: wt::TitleData,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(convert::title_data_from_wit(&title)
            .map_err(|e| invalid_component(&e))
            .and_then(|title| session.send_title(title).map_err(player_error)))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<wl::LimboSession>,
        message: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(parse_text(&message)
            .and_then(|message| session.send_action_bar(message).map_err(player_error)))
    }

    async fn complete(
        &mut self,
        self_: Resource<wl::LimboSession>,
        outcome: wl::HandlerResult,
    ) -> wasmtime::Result<HostResult<()>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(convert::complete_result_from_wit(&outcome)
            .map(|outcome| session.complete(outcome))
            .map_err(|e| invalid_component(&e)))
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
        message: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(parse_text(&message)
            .and_then(|message| handle.send_message(message).map_err(player_error)))
    }

    async fn send_title(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        title: wt::TitleData,
    ) -> wasmtime::Result<HostResult<()>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(convert::title_data_from_wit(&title)
            .map_err(|e| invalid_component(&e))
            .and_then(|title| handle.send_title(title).map_err(player_error)))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        message: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(parse_text(&message)
            .and_then(|message| handle.send_action_bar(message).map_err(player_error)))
    }

    async fn complete(
        &mut self,
        self_: Resource<wl::LimboSessionHandle>,
        outcome: wl::HandlerResult,
    ) -> wasmtime::Result<HostResult<()>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(convert::complete_result_from_wit(&outcome)
            .map(|outcome| handle.complete(outcome))
            .map_err(|e| invalid_component(&e)))
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
