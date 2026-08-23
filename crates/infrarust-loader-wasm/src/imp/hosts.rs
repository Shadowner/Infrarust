use std::future::Future;
use std::time::Duration;

use infrarust_api::error::ServiceError;
use infrarust_api::filter::{FilterMetadata, FilterPriority};
use infrarust_api::permissions::Capability;
use infrarust_api::services::scheduler::TaskHandle;
use infrarust_api::types::{PlayerId, ServerId};
use tokio::time::timeout;
use wasmtime::component::Resource;

use crate::bindings::infrarust::plugin::player_registry::Player as PlayerHandle;
use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, limbo, log,
    player_registry, scheduler, server_manager, types as wt,
};
use crate::consts::{HOST_CALL_TIMEOUT, PLAYER_SWITCH_TIMEOUT};
use crate::store_state::PluginStoreState;
use crate::{convert, dispatch, proxies};

async fn await_service<T>(
    fut: impl Future<Output = Result<T, ServiceError>> + Send,
) -> Result<T, wt::ServiceError> {
    match timeout(HOST_CALL_TIMEOUT, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(convert::service_error_to_wit(e)),
        Err(_) => Err(wt::ServiceError::Unavailable(
            "host call timed out".to_string(),
        )),
    }
}

impl log::Host for PluginStoreState {
    async fn trace(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::trace!(plugin = %self.plugin_id, "{message}");
        Ok(())
    }
    async fn debug(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::debug!(plugin = %self.plugin_id, "{message}");
        Ok(())
    }
    async fn info(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::info!(plugin = %self.plugin_id, "{message}");
        Ok(())
    }
    async fn warn(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::warn!(plugin = %self.plugin_id, "{message}");
        Ok(())
    }
    async fn error(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::error!(plugin = %self.plugin_id, "{message}");
        Ok(())
    }
}

impl event_bus::Host for PluginStoreState {
    async fn subscribe(
        &mut self,
        kind: event_bus::EventKind,
        priority: wt::EventPriority,
    ) -> wasmtime::Result<u64> {
        let instance = self.instance_ref();
        let ctx = self.require_ctx()?;
        let native_priority = dispatch::priority_from_wit(priority);
        let listener_id = self.mint_listener_id();
        let handle = dispatch::register_event_handler(
            ctx.as_ref(),
            instance,
            kind,
            native_priority,
            listener_id,
        );
        self.record_listener(listener_id, handle);
        Ok(listener_id)
    }

    async fn unsubscribe(&mut self, handle: u64) -> wasmtime::Result<()> {
        if let Some(native) = self.take_listener(handle)
            && let Some(ctx) = self.ctx()
        {
            ctx.event_bus().unsubscribe(native);
        }
        Ok(())
    }
}

impl player_registry::Host for PluginStoreState {
    async fn get_player(
        &mut self,
        username: String,
    ) -> wasmtime::Result<Option<Resource<PlayerHandle>>> {
        let ctx = self.require_ctx()?;
        match ctx.player_registry().get_player(&username) {
            Some(p) => Ok(Some(self.push_player(p)?)),
            None => Ok(None),
        }
    }

    async fn get_player_by_uuid(
        &mut self,
        player_uuid: String,
    ) -> wasmtime::Result<Option<Resource<PlayerHandle>>> {
        let uuid = uuid::Uuid::parse_str(&player_uuid)
            .map_err(|e| wasmtime::Error::msg(format!("invalid uuid {player_uuid:?}: {e}")))?;
        let ctx = self.require_ctx()?;
        match ctx.player_registry().get_player_by_uuid(&uuid) {
            Some(p) => Ok(Some(self.push_player(p)?)),
            None => Ok(None),
        }
    }

    async fn get_player_by_id(
        &mut self,
        id: u64,
    ) -> wasmtime::Result<Option<Resource<PlayerHandle>>> {
        let ctx = self.require_ctx()?;
        match ctx.player_registry().get_player_by_id(PlayerId::new(id)) {
            Some(p) => Ok(Some(self.push_player(p)?)),
            None => Ok(None),
        }
    }

    async fn get_players_on_server(
        &mut self,
        server: String,
    ) -> wasmtime::Result<Vec<Resource<PlayerHandle>>> {
        let ctx = self.require_ctx()?;
        let players = ctx
            .player_registry()
            .get_players_on_server(&ServerId::from(server));
        let mut out = Vec::with_capacity(players.len());
        for p in players {
            out.push(self.push_player(p)?);
        }
        Ok(out)
    }

    async fn get_all_players(&mut self) -> wasmtime::Result<Vec<Resource<PlayerHandle>>> {
        let ctx = self.require_ctx()?;
        let players = ctx.player_registry().get_all_players();
        let mut out = Vec::with_capacity(players.len());
        for p in players {
            out.push(self.push_player(p)?);
        }
        Ok(out)
    }

    async fn online_count(&mut self) -> wasmtime::Result<u32> {
        let ctx = self.require_ctx()?;
        Ok(u32::try_from(ctx.player_registry().online_count()).unwrap_or(u32::MAX))
    }

    async fn online_count_on(&mut self, server: String) -> wasmtime::Result<u32> {
        let ctx = self.require_ctx()?;
        let n = ctx
            .player_registry()
            .online_count_on(&ServerId::from(server));
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}

impl player_registry::HostPlayer for PluginStoreState {
    async fn id(&mut self, self_: Resource<PlayerHandle>) -> wasmtime::Result<u64> {
        Ok(self.resolve_player(&self_)?.id().as_u64())
    }

    async fn profile(
        &mut self,
        self_: Resource<PlayerHandle>,
    ) -> wasmtime::Result<wt::GameProfile> {
        let p = self.resolve_player(&self_)?;
        Ok(convert::game_profile_to_wit(p.profile()))
    }

    async fn protocol_version(&mut self, self_: Resource<PlayerHandle>) -> wasmtime::Result<i32> {
        Ok(self.resolve_player(&self_)?.protocol_version().raw())
    }

    async fn remote_addr(&mut self, self_: Resource<PlayerHandle>) -> wasmtime::Result<String> {
        Ok(self.resolve_player(&self_)?.remote_addr().to_string())
    }

    async fn current_server(
        &mut self,
        self_: Resource<PlayerHandle>,
    ) -> wasmtime::Result<Option<String>> {
        Ok(self
            .resolve_player(&self_)?
            .current_server()
            .map(|s| s.as_str().to_string()))
    }

    async fn is_connected(&mut self, self_: Resource<PlayerHandle>) -> wasmtime::Result<bool> {
        Ok(self.resolve_player(&self_)?.is_connected())
    }

    async fn is_active(&mut self, self_: Resource<PlayerHandle>) -> wasmtime::Result<bool> {
        Ok(self.resolve_player(&self_)?.is_active())
    }

    async fn disconnect(
        &mut self,
        self_: Resource<PlayerHandle>,
        reason: String,
    ) -> wasmtime::Result<()> {
        let player = self.resolve_player(&self_)?;
        let component = convert::component_from_wit(&reason);
        let plugin_id = self.plugin_id.clone();
        tokio::spawn(async move {
            let player_id = player.id().as_u64();
            if timeout(HOST_CALL_TIMEOUT, player.disconnect(component))
                .await
                .is_err()
            {
                tracing::warn!(plugin = %plugin_id, player = player_id,
                    "player disconnect requested by plugin timed out");
            }
        });
        Ok(())
    }

    async fn send_message(
        &mut self,
        self_: Resource<PlayerHandle>,
        message: String,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let player = self.resolve_player(&self_)?;
        let component = convert::component_from_wit(&message);
        Ok(player
            .send_message(component)
            .map_err(convert::player_error_to_wit))
    }

    async fn send_title(
        &mut self,
        self_: Resource<PlayerHandle>,
        title: wt::TitleData,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let player = self.resolve_player(&self_)?;
        Ok(player
            .send_title(convert::title_data_from_wit(title))
            .map_err(convert::player_error_to_wit))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<PlayerHandle>,
        message: String,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let player = self.resolve_player(&self_)?;
        Ok(player
            .send_action_bar(convert::component_from_wit(&message))
            .map_err(convert::player_error_to_wit))
    }

    async fn send_packet(
        &mut self,
        self_: Resource<PlayerHandle>,
        packet: wt::RawPacket,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        if !self.capabilities().has(Capability::RawPacket) {
            return Ok(Err(wt::PlayerError::SendFailed(
                "missing capability: raw-packet".to_string(),
            )));
        }
        let player = self.resolve_player(&self_)?;
        Ok(player
            .send_packet(convert::raw_packet_from_wit(packet))
            .map_err(convert::player_error_to_wit))
    }

    async fn switch_server(
        &mut self,
        self_: Resource<PlayerHandle>,
        target: String,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let player = self.resolve_player(&self_)?;
        let target = ServerId::from(target);
        Ok(
            match timeout(PLAYER_SWITCH_TIMEOUT, player.switch_server(target)).await {
                Ok(result) => result.map_err(convert::player_error_to_wit),
                Err(_) => Err(wt::PlayerError::SwitchFailed(
                    "host call timed out".to_string(),
                )),
            },
        )
    }

    async fn is_online_mode(&mut self, self_: Resource<PlayerHandle>) -> wasmtime::Result<bool> {
        Ok(self.resolve_player(&self_)?.is_online_mode())
    }

    async fn permission_level(
        &mut self,
        self_: Resource<PlayerHandle>,
    ) -> wasmtime::Result<wt::PermissionLevel> {
        let p = self.resolve_player(&self_)?;
        Ok(convert::permission_level_to_wit(p.permission_level()))
    }

    async fn has_permission(
        &mut self,
        self_: Resource<PlayerHandle>,
        permission: String,
    ) -> wasmtime::Result<bool> {
        Ok(self.resolve_player(&self_)?.has_permission(&permission))
    }

    async fn connected_at(&mut self, self_: Resource<PlayerHandle>) -> wasmtime::Result<u64> {
        let p = self.resolve_player(&self_)?;
        Ok(convert::system_time_to_millis(p.connected_at()))
    }

    async fn drop(&mut self, rep: Resource<PlayerHandle>) -> wasmtime::Result<()> {
        self.drop_player(rep)
    }
}

impl server_manager::Host for PluginStoreState {
    async fn get_state(&mut self, server: String) -> wasmtime::Result<Option<wt::ServerState>> {
        let ctx = self.require_ctx()?;
        Ok(ctx
            .server_manager()
            .get_state(&ServerId::from(server))
            .map(convert::server_state_to_wit))
    }

    async fn start(&mut self, server: String) -> wasmtime::Result<Result<(), wt::ServiceError>> {
        let ctx = self.require_ctx()?;
        let sid = ServerId::from(server);
        Ok(await_service(ctx.server_manager().start(&sid)).await)
    }

    async fn stop(&mut self, server: String) -> wasmtime::Result<Result<(), wt::ServiceError>> {
        let ctx = self.require_ctx()?;
        let sid = ServerId::from(server);
        Ok(await_service(ctx.server_manager().stop(&sid)).await)
    }

    async fn get_all_servers(&mut self) -> wasmtime::Result<Vec<(String, wt::ServerState)>> {
        let ctx = self.require_ctx()?;
        Ok(ctx
            .server_manager()
            .get_all_servers()
            .into_iter()
            .map(|(id, st)| (id.as_str().to_string(), convert::server_state_to_wit(st)))
            .collect())
    }
}

impl ban_service::Host for PluginStoreState {
    async fn ban(
        &mut self,
        target: wt::BanTarget,
        reason: Option<String>,
        duration_ms: Option<u64>,
    ) -> wasmtime::Result<Result<(), wt::ServiceError>> {
        let ctx = self.require_ctx()?;
        let Some(native_target) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        let duration = duration_ms.map(Duration::from_millis);
        Ok(await_service(ctx.ban_service().ban(native_target, reason, duration)).await)
    }

    async fn unban(
        &mut self,
        target: wt::BanTarget,
    ) -> wasmtime::Result<Result<bool, wt::ServiceError>> {
        let ctx = self.require_ctx()?;
        let Some(t) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        Ok(await_service(ctx.ban_service().unban(&t)).await)
    }

    async fn is_banned(
        &mut self,
        target: wt::BanTarget,
    ) -> wasmtime::Result<Result<bool, wt::ServiceError>> {
        let ctx = self.require_ctx()?;
        let Some(t) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        Ok(await_service(ctx.ban_service().is_banned(&t)).await)
    }

    async fn get_ban(
        &mut self,
        target: wt::BanTarget,
    ) -> wasmtime::Result<Result<Option<wt::BanEntry>, wt::ServiceError>> {
        let ctx = self.require_ctx()?;
        let Some(t) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        Ok(await_service(ctx.ban_service().get_ban(&t))
            .await
            .map(|opt| opt.as_ref().map(convert::ban_entry_to_wit)))
    }

    async fn get_all_bans(
        &mut self,
    ) -> wasmtime::Result<Result<Vec<wt::BanEntry>, wt::ServiceError>> {
        let ctx = self.require_ctx()?;
        Ok(await_service(ctx.ban_service().get_all_bans())
            .await
            .map(|bans| bans.iter().map(convert::ban_entry_to_wit).collect()))
    }
}

impl config_service::Host for PluginStoreState {
    async fn get_server_config(
        &mut self,
        server: String,
    ) -> wasmtime::Result<Option<wt::ServerConfig>> {
        let ctx = self.require_ctx()?;
        Ok(ctx
            .config_service()
            .get_server_config(&ServerId::from(server))
            .as_ref()
            .map(convert::server_config_to_wit))
    }

    async fn get_all_server_configs(&mut self) -> wasmtime::Result<Vec<wt::ServerConfig>> {
        let ctx = self.require_ctx()?;
        Ok(ctx
            .config_service()
            .get_all_server_configs()
            .iter()
            .map(convert::server_config_to_wit)
            .collect())
    }

    async fn get_value(&mut self, key: String) -> wasmtime::Result<Option<String>> {
        let ctx = self.require_ctx()?;
        Ok(ctx.config_service().get_value(&key))
    }
}

impl command_manager::Host for PluginStoreState {
    async fn register(
        &mut self,
        name: String,
        aliases: Vec<String>,
        description: String,
        callback_id: u64,
    ) -> wasmtime::Result<()> {
        let instance = self.instance_ref();
        let ctx = self.require_ctx()?;
        let handler = Box::new(proxies::WasmCommandHandler::new(callback_id, instance));
        let alias_refs: Vec<&str> = aliases.iter().map(String::as_str).collect();
        ctx.command_manager()
            .register(&name, &alias_refs, &description, handler);
        Ok(())
    }

    async fn unregister(&mut self, name: String) -> wasmtime::Result<()> {
        if let Some(ctx) = self.ctx() {
            ctx.command_manager().unregister(&name);
        }
        Ok(())
    }
}

fn filter_priority_from_u8(p: u8) -> FilterPriority {
    match p {
        0 => FilterPriority::First,
        1 => FilterPriority::Early,
        2 => FilterPriority::Normal,
        3 => FilterPriority::Late,
        _ => FilterPriority::Last,
    }
}

impl codec_registry::Host for PluginStoreState {
    async fn register_codec_filter(
        &mut self,
        metadata: codec_registry::CodecFilterMetadata,
        factory: u64,
    ) -> wasmtime::Result<()> {
        let Some(instantiator) = self.codec_instantiator().cloned() else {
            return Err(wasmtime::Error::msg(
                "codec instantiator unavailable (register-codec-filter called off the load path)",
            ));
        };
        let ctx = self.require_ctx()?;
        let Some(registry) = ctx.codec_filters() else {
            return Err(wasmtime::Error::msg(
                "host context exposes no codec filter registry",
            ));
        };
        let native_meta = FilterMetadata {
            id: metadata.id,
            priority: filter_priority_from_u8(metadata.priority),
            after: metadata.after,
            before: metadata.before,
        };
        registry.register(Box::new(crate::codec::WasmCodecFilterFactory::new(
            instantiator,
            factory,
            native_meta,
        )));
        Ok(())
    }

    async fn unregister_codec_filter(&mut self, id: String) -> wasmtime::Result<()> {
        if let Some(ctx) = self.ctx()
            && let Some(registry) = ctx.codec_filters()
        {
            registry.unregister(&id);
        }
        Ok(())
    }
}
impl scheduler::Host for PluginStoreState {
    async fn delay(&mut self, after_ms: u64, callback_id: u64) -> wasmtime::Result<u64> {
        let instance = self.instance_ref();
        let ctx = self.require_ctx()?;
        let handle = ctx.scheduler().delay(
            Duration::from_millis(after_ms),
            Box::new(move || {
                proxies::dispatch_scheduled_task(instance, callback_id);
            }),
        );
        Ok(handle.as_u64())
    }

    async fn interval(&mut self, period_ms: u64, callback_id: u64) -> wasmtime::Result<u64> {
        let instance = self.instance_ref();
        let ctx = self.require_ctx()?;
        let handle = ctx.scheduler().interval(
            Duration::from_millis(period_ms),
            Box::new(move || {
                proxies::dispatch_scheduled_task(instance.clone(), callback_id);
            }),
        );
        Ok(handle.as_u64())
    }

    async fn cancel(&mut self, handle: u64) -> wasmtime::Result<()> {
        if let Some(ctx) = self.ctx() {
            ctx.scheduler().cancel(TaskHandle::new(handle));
        }
        Ok(())
    }
}
impl limbo::Host for PluginStoreState {
    async fn register_limbo_handler(&mut self, name: String, handler: u64) -> wasmtime::Result<()> {
        if !self.capabilities().has(Capability::Limbo) {
            tracing::warn!(plugin = %self.plugin_id, handler = %name,
                "register_limbo_handler denied: missing Limbo capability");
            return Ok(());
        }
        let instance = self.instance_ref();
        let ctx = self.require_ctx()?;
        ctx.register_limbo_handler(Box::new(crate::limbo::WasmLimboHandler::new(
            handler, name, instance,
        )));
        Ok(())
    }
}

impl limbo::HostLimboSession for PluginStoreState {
    async fn player_id(&mut self, self_: Resource<limbo::LimboSession>) -> wasmtime::Result<u64> {
        Ok(self.resolve_limbo_session(&self_)?.player_id().as_u64())
    }

    async fn profile(
        &mut self,
        self_: Resource<limbo::LimboSession>,
    ) -> wasmtime::Result<wt::GameProfile> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(convert::game_profile_to_wit(session.profile()))
    }

    async fn entry_context(
        &mut self,
        self_: Resource<limbo::LimboSession>,
    ) -> wasmtime::Result<limbo::LimboEntryContext> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(convert::limbo_entry_context_to_wit(session.entry_context()))
    }

    async fn send_message(
        &mut self,
        self_: Resource<limbo::LimboSession>,
        message: String,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(session
            .send_message(convert::component_from_wit(&message))
            .map_err(convert::player_error_to_wit))
    }

    async fn send_title(
        &mut self,
        self_: Resource<limbo::LimboSession>,
        title: wt::TitleData,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(session
            .send_title(convert::title_data_from_wit(title))
            .map_err(convert::player_error_to_wit))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<limbo::LimboSession>,
        message: String,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let session = self.resolve_limbo_session(&self_)?;
        Ok(session
            .send_action_bar(convert::component_from_wit(&message))
            .map_err(convert::player_error_to_wit))
    }

    async fn complete(
        &mut self,
        self_: Resource<limbo::LimboSession>,
        outcome: limbo::HandlerResult,
    ) -> wasmtime::Result<()> {
        let session = self.resolve_limbo_session(&self_)?;
        session.complete(convert::complete_result_from_wit(outcome));
        Ok(())
    }

    async fn acquire_handle(
        &mut self,
        self_: Resource<limbo::LimboSession>,
    ) -> wasmtime::Result<Resource<limbo::LimboSessionHandle>> {
        let session = self.resolve_limbo_session(&self_)?;
        self.push_limbo_session_handle(session.handle())
    }

    async fn drop(&mut self, rep: Resource<limbo::LimboSession>) -> wasmtime::Result<()> {
        let _ = self.drop_limbo_session(rep);
        Ok(())
    }
}

impl limbo::HostLimboSessionHandle for PluginStoreState {
    async fn player_id(
        &mut self,
        self_: Resource<limbo::LimboSessionHandle>,
    ) -> wasmtime::Result<u64> {
        Ok(self
            .resolve_limbo_session_handle(&self_)?
            .player_id()
            .as_u64())
    }

    async fn send_message(
        &mut self,
        self_: Resource<limbo::LimboSessionHandle>,
        message: String,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(handle
            .send_message(convert::component_from_wit(&message))
            .map_err(convert::player_error_to_wit))
    }

    async fn send_title(
        &mut self,
        self_: Resource<limbo::LimboSessionHandle>,
        title: wt::TitleData,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(handle
            .send_title(convert::title_data_from_wit(title))
            .map_err(convert::player_error_to_wit))
    }

    async fn send_action_bar(
        &mut self,
        self_: Resource<limbo::LimboSessionHandle>,
        message: String,
    ) -> wasmtime::Result<Result<(), wt::PlayerError>> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        Ok(handle
            .send_action_bar(convert::component_from_wit(&message))
            .map_err(convert::player_error_to_wit))
    }

    async fn complete(
        &mut self,
        self_: Resource<limbo::LimboSessionHandle>,
        outcome: limbo::HandlerResult,
    ) -> wasmtime::Result<()> {
        let handle = self.resolve_limbo_session_handle(&self_)?;
        handle.complete(convert::complete_result_from_wit(outcome));
        Ok(())
    }

    async fn cancelled(
        &mut self,
        self_: Resource<limbo::LimboSessionHandle>,
    ) -> wasmtime::Result<bool> {
        Ok(self
            .resolve_limbo_session_handle(&self_)?
            .cancellation_token()
            .is_cancelled())
    }

    async fn drop(&mut self, rep: Resource<limbo::LimboSessionHandle>) -> wasmtime::Result<()> {
        let _ = self.drop_limbo_session_handle(rep);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime};

    use infrarust_api::error::PlayerError;
    use infrarust_api::event::BoxFuture;
    use infrarust_api::permissions::PermissionLevel;
    use infrarust_api::player::Player;
    use infrarust_api::types::{
        Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, TitleData,
    };

    use super::*;
    use crate::bindings::infrarust::plugin::player_registry::{Host as _, HostPlayer as _};
    use crate::store_state::build_probe_state;

    #[tokio::test]
    async fn get_player_by_uuid_traps_on_malformed_uuid() {
        let mut state = build_probe_state("test".to_string());
        let err = state
            .get_player_by_uuid("not-a-uuid".to_string())
            .await
            .expect_err("malformed uuid must trap, not read as player-not-found");
        assert!(err.to_string().contains("invalid uuid"), "{err}");
    }

    struct StalledPlayer {
        profile: GameProfile,
    }

    impl StalledPlayer {
        fn new() -> Self {
            Self {
                profile: GameProfile {
                    uuid: uuid::Uuid::nil(),
                    username: "tester".to_string(),
                    properties: vec![],
                },
            }
        }
    }

    impl infrarust_api::player::private::Sealed for StalledPlayer {}

    impl Player for StalledPlayer {
        fn id(&self) -> PlayerId {
            PlayerId::new(1)
        }
        fn profile(&self) -> &GameProfile {
            &self.profile
        }
        fn protocol_version(&self) -> ProtocolVersion {
            ProtocolVersion::MINECRAFT_1_21
        }
        fn remote_addr(&self) -> SocketAddr {
            SocketAddr::from(([127, 0, 0, 1], 0))
        }
        fn current_server(&self) -> Option<ServerId> {
            None
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn is_active(&self) -> bool {
            true
        }
        fn disconnect(&self, _reason: Component) -> BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
        fn send_message(&self, _message: Component) -> Result<(), PlayerError> {
            Ok(())
        }
        fn send_title(&self, _title: TitleData) -> Result<(), PlayerError> {
            Ok(())
        }
        fn send_action_bar(&self, _message: Component) -> Result<(), PlayerError> {
            Ok(())
        }
        fn send_packet(&self, _packet: RawPacket) -> Result<(), PlayerError> {
            Ok(())
        }
        fn switch_server(&self, _target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
            Box::pin(std::future::pending())
        }
        fn is_online_mode(&self) -> bool {
            true
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::Player
        }
        fn has_permission(&self, _permission: &str) -> bool {
            false
        }
        fn connected_at(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH
        }
    }

    #[tokio::test]
    async fn switch_server_gives_up_long_before_the_host_call_timeout() {
        let mut state = build_probe_state("test".to_string());
        let handle = state
            .push_player(Arc::new(StalledPlayer::new()))
            .expect("the resource table accepts a player");

        let started = Instant::now();
        let result = state
            .switch_server(handle, "lobby".to_string())
            .await
            .expect("a saturated session is a player-error, not a trap");
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(wt::PlayerError::SwitchFailed(_))),
            "{result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "held the instance lock for {elapsed:?}"
        );
    }
}
