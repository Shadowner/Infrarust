use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::command::CommandSpec;
use infrarust_api::error::ServiceError;
use infrarust_api::filter::{FilterMetadata, FilterPriority};
use infrarust_api::permissions::Capability;
use infrarust_api::services::scheduler::TaskHandle;
use infrarust_api::types::{PlayerId, ServerId};
use tokio::time::timeout;
use wasmtime::component::Resource;

use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::player_registry::Player as PlayerHandle;
use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, limbo, log,
    player_registry, scheduler, server_manager, types as wt,
};
use crate::consts::PLAYER_SWITCH_TIMEOUT;
use crate::deadline::HostCallLimit;
use crate::registrations::Bound;
use crate::store_state::PluginStoreState;
use crate::{convert, dispatch, proxies};

async fn await_service<T>(
    limit: HostCallLimit,
    fut: impl Future<Output = Result<T, ServiceError>> + Send,
) -> Result<T, wt::ServiceError> {
    match limit.run(fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(convert::service_error_to_wit(e)),
        Err(expired) => Err(wt::ServiceError::Unavailable(expired)),
    }
}

fn missing_capability(capability: Capability) -> String {
    format!("missing capability: {}", capability.to_kebab())
}

impl PluginStoreState {
    fn service_call_limit(&self) -> HostCallLimit {
        self.host_call_limit(self.host_call_timeout())
    }

    fn lacks(&mut self, capability: Capability, call: &'static str) -> bool {
        if self.capabilities().has(capability) {
            return false;
        }
        self.report_denied(capability, call);
        true
    }

    fn refused_service(
        &mut self,
        capability: Capability,
        call: &'static str,
    ) -> Option<wt::ServiceError> {
        self.lacks(capability, call)
            .then(|| wt::ServiceError::OperationFailed(missing_capability(capability)))
    }

    fn refused_player(
        &mut self,
        capability: Capability,
        call: &'static str,
        error: impl FnOnce(String) -> wt::PlayerError,
    ) -> Option<wt::PlayerError> {
        self.lacks(capability, call)
            .then(|| error(missing_capability(capability)))
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
        if self.lacks(Capability::EventBus, "event-bus.subscribe")
            || (matches!(kind, event_bus::EventKind::RawPacket)
                && self.lacks(Capability::RawPacket, "event-bus.subscribe(raw-packet)"))
        {
            return Ok(self.mint_listener_id());
        }
        let instance = self.instance_ref(CallKind::Event);
        let ctx = self.require_ctx()?;
        let native_priority = dispatch::priority_from_wit(priority);
        let listener_id = self.mint_listener_id();
        match dispatch::register_event_handler(
            ctx.event_bus(),
            instance,
            kind,
            native_priority,
            listener_id,
        ) {
            Some(handle) => self.record_listener(listener_id, handle),
            None => tracing::warn!(
                plugin = %self.plugin_id,
                "raw packets are not available to WASM plugins in this contract version; the raw-packet subscription has no listener"
            ),
        }
        Ok(listener_id)
    }

    async fn unsubscribe(&mut self, handle: u64) -> wasmtime::Result<()> {
        if self.lacks(Capability::EventBus, "event-bus.unsubscribe") {
            return Ok(());
        }
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
        if self.lacks(Capability::PlayerRead, "player-registry.get-player") {
            return Ok(None);
        }
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
        if self.lacks(Capability::PlayerRead, "player-registry.get-player-by-uuid") {
            return Ok(None);
        }
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
        if self.lacks(Capability::PlayerRead, "player-registry.get-player-by-id") {
            return Ok(None);
        }
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
        if self.lacks(
            Capability::PlayerRead,
            "player-registry.get-players-on-server",
        ) {
            return Ok(Vec::new());
        }
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
        if self.lacks(Capability::PlayerRead, "player-registry.get-all-players") {
            return Ok(Vec::new());
        }
        let ctx = self.require_ctx()?;
        let players = ctx.player_registry().get_all_players();
        let mut out = Vec::with_capacity(players.len());
        for p in players {
            out.push(self.push_player(p)?);
        }
        Ok(out)
    }

    async fn online_count(&mut self) -> wasmtime::Result<u32> {
        if self.lacks(Capability::PlayerRead, "player-registry.online-count") {
            return Ok(0);
        }
        let ctx = self.require_ctx()?;
        Ok(u32::try_from(ctx.player_registry().online_count()).unwrap_or(u32::MAX))
    }

    async fn online_count_on(&mut self, server: String) -> wasmtime::Result<u32> {
        if self.lacks(Capability::PlayerRead, "player-registry.online-count-on") {
            return Ok(0);
        }
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
        if self.lacks(Capability::PlayerWrite, "player.disconnect") {
            return Ok(());
        }
        let player = self.resolve_player(&self_)?;
        let component = convert::component_from_wit(&reason);
        let plugin_id = self.plugin_id.clone();
        let limit = self.host_call_timeout();
        tokio::spawn(async move {
            let player_id = player.id().as_u64();
            if timeout(limit, player.disconnect(component)).await.is_err() {
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
        if let Some(denied) = self.refused_player(
            Capability::PlayerWrite,
            "player.send-message",
            wt::PlayerError::SendFailed,
        ) {
            return Ok(Err(denied));
        }
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
        if let Some(denied) = self.refused_player(
            Capability::PlayerWrite,
            "player.send-title",
            wt::PlayerError::SendFailed,
        ) {
            return Ok(Err(denied));
        }
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
        if let Some(denied) = self.refused_player(
            Capability::PlayerWrite,
            "player.send-action-bar",
            wt::PlayerError::SendFailed,
        ) {
            return Ok(Err(denied));
        }
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
        if let Some(denied) = self.refused_player(
            Capability::RawPacket,
            "player.send-packet",
            wt::PlayerError::SendFailed,
        ) {
            return Ok(Err(denied));
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
        if let Some(denied) = self.refused_player(
            Capability::PlayerWrite,
            "player.switch-server",
            wt::PlayerError::SwitchFailed,
        ) {
            return Ok(Err(denied));
        }
        let player = self.resolve_player(&self_)?;
        let target = ServerId::from(target);
        let limit = self.host_call_limit(PLAYER_SWITCH_TIMEOUT);
        Ok(match limit.run(player.switch_server(target)).await {
            Ok(result) => result.map_err(convert::player_error_to_wit),
            Err(expired) => Err(wt::PlayerError::SwitchFailed(expired)),
        })
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
        if self.lacks(Capability::ServerManage, "server-manager.get-state") {
            return Ok(None);
        }
        let ctx = self.require_ctx()?;
        Ok(ctx
            .server_manager()
            .get_state(&ServerId::from(server))
            .map(convert::server_state_to_wit))
    }

    async fn start(&mut self, server: String) -> wasmtime::Result<Result<(), wt::ServiceError>> {
        if let Some(denied) = self.refused_service(Capability::ServerManage, "server-manager.start")
        {
            return Ok(Err(denied));
        }
        let ctx = self.require_ctx()?;
        let sid = ServerId::from(server);
        Ok(await_service(self.service_call_limit(), ctx.server_manager().start(&sid)).await)
    }

    async fn stop(&mut self, server: String) -> wasmtime::Result<Result<(), wt::ServiceError>> {
        if let Some(denied) = self.refused_service(Capability::ServerManage, "server-manager.stop")
        {
            return Ok(Err(denied));
        }
        let ctx = self.require_ctx()?;
        let sid = ServerId::from(server);
        Ok(await_service(self.service_call_limit(), ctx.server_manager().stop(&sid)).await)
    }

    async fn get_all_servers(&mut self) -> wasmtime::Result<Vec<(String, wt::ServerState)>> {
        if self.lacks(Capability::ServerManage, "server-manager.get-all-servers") {
            return Ok(Vec::new());
        }
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
        if let Some(denied) = self.refused_service(Capability::Ban, "ban-service.ban") {
            return Ok(Err(denied));
        }
        let ctx = self.require_ctx()?;
        let Some(native_target) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        let duration = duration_ms.map(Duration::from_millis);
        Ok(await_service(
            self.service_call_limit(),
            ctx.ban_service().ban(native_target, reason, duration),
        )
        .await)
    }

    async fn unban(
        &mut self,
        target: wt::BanTarget,
    ) -> wasmtime::Result<Result<bool, wt::ServiceError>> {
        if let Some(denied) = self.refused_service(Capability::Ban, "ban-service.unban") {
            return Ok(Err(denied));
        }
        let ctx = self.require_ctx()?;
        let Some(t) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        Ok(await_service(self.service_call_limit(), ctx.ban_service().unban(&t)).await)
    }

    async fn is_banned(
        &mut self,
        target: wt::BanTarget,
    ) -> wasmtime::Result<Result<bool, wt::ServiceError>> {
        if let Some(denied) = self.refused_service(Capability::Ban, "ban-service.is-banned") {
            return Ok(Err(denied));
        }
        let ctx = self.require_ctx()?;
        let Some(t) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        Ok(await_service(self.service_call_limit(), ctx.ban_service().is_banned(&t)).await)
    }

    async fn get_ban(
        &mut self,
        target: wt::BanTarget,
    ) -> wasmtime::Result<Result<Option<wt::BanEntry>, wt::ServiceError>> {
        if let Some(denied) = self.refused_service(Capability::Ban, "ban-service.get-ban") {
            return Ok(Err(denied));
        }
        let ctx = self.require_ctx()?;
        let Some(t) = convert::ban_target_from_wit(&target) else {
            return Ok(Err(wt::ServiceError::OperationFailed(
                "invalid ban target".to_string(),
            )));
        };
        Ok(
            await_service(self.service_call_limit(), ctx.ban_service().get_ban(&t))
                .await
                .map(|opt| opt.as_ref().map(convert::ban_entry_to_wit)),
        )
    }

    async fn get_all_bans(
        &mut self,
    ) -> wasmtime::Result<Result<Vec<wt::BanEntry>, wt::ServiceError>> {
        if let Some(denied) = self.refused_service(Capability::Ban, "ban-service.get-all-bans") {
            return Ok(Err(denied));
        }
        let ctx = self.require_ctx()?;
        Ok(
            await_service(self.service_call_limit(), ctx.ban_service().get_all_bans())
                .await
                .map(|bans| bans.iter().map(convert::ban_entry_to_wit).collect()),
        )
    }
}

impl config_service::Host for PluginStoreState {
    async fn get_server_config(
        &mut self,
        server: String,
    ) -> wasmtime::Result<Option<wt::ServerConfig>> {
        if self.lacks(Capability::ConfigRead, "config-service.get-server-config") {
            return Ok(None);
        }
        let ctx = self.require_ctx()?;
        Ok(ctx
            .config_service()
            .get_server_config(&ServerId::from(server))
            .as_ref()
            .map(convert::server_config_to_wit))
    }

    async fn get_all_server_configs(&mut self) -> wasmtime::Result<Vec<wt::ServerConfig>> {
        if self.lacks(
            Capability::ConfigRead,
            "config-service.get-all-server-configs",
        ) {
            return Ok(Vec::new());
        }
        let ctx = self.require_ctx()?;
        Ok(ctx
            .config_service()
            .get_all_server_configs()
            .iter()
            .map(convert::server_config_to_wit)
            .collect())
    }

    async fn get_value(&mut self, key: String) -> wasmtime::Result<Option<String>> {
        if self.lacks(Capability::ConfigRead, "config-service.get-value") {
            return Ok(None);
        }
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
        if self.lacks(Capability::Command, "command-manager.register") {
            return Ok(());
        }
        let instance = self.instance_ref(CallKind::Callback).any_generation();
        let ctx = self.require_ctx()?;
        let Bound::Fresh(binding) =
            self.registrations()
                .bind_command(&name, self.generation(), callback_id)
        else {
            return Ok(());
        };
        let handler = Box::new(proxies::WasmCommandHandler::new(binding, instance));
        let spec = CommandSpec::new(name.as_str())
            .aliases(aliases)
            .description(description);
        match ctx.command_manager().register(spec, handler) {
            Ok(registration) if !registration.rejected_aliases.is_empty() => {
                let reason = format!(
                    "aliases {:?} are taken or invalid and were skipped",
                    registration.rejected_aliases
                );
                self.report_command_refusal(&name, &reason);
            }
            Ok(_) => {}
            Err(e) => {
                self.registrations().unbind_command(&name);
                self.report_command_refusal(&name, &format!("refused, {e}"));
            }
        }
        Ok(())
    }

    async fn unregister(&mut self, name: String) -> wasmtime::Result<()> {
        if self.lacks(Capability::Command, "command-manager.unregister") {
            return Ok(());
        }
        let Some(ctx) = self.ctx().cloned() else {
            self.registrations().unbind_command(&name);
            return Ok(());
        };
        match ctx.command_manager().unregister(&name) {
            Ok(()) => self.registrations().unbind_command(&name),
            Err(e) => self.report_command_refusal(&name, &format!("unregister refused, {e}")),
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
        if self.lacks(
            Capability::CodecFilter,
            "codec-registry.register-codec-filter",
        ) {
            return Ok(());
        }
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
        if self.lacks(
            Capability::CodecFilter,
            "codec-registry.unregister-codec-filter",
        ) {
            return Ok(());
        }
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
        if self.lacks(Capability::Scheduler, "scheduler.delay") {
            return Ok(0);
        }
        let instance = self.instance_ref(CallKind::Callback);
        let ctx = self.require_ctx()?;
        let handle = ctx.scheduler().delay(
            Duration::from_millis(after_ms),
            Box::new(move || {
                proxies::dispatch_scheduled_task(instance, callback_id);
            }),
        );
        self.record_task(handle.as_u64());
        Ok(handle.as_u64())
    }

    async fn interval(&mut self, period_ms: u64, callback_id: u64) -> wasmtime::Result<u64> {
        if self.lacks(Capability::Scheduler, "scheduler.interval") {
            return Ok(0);
        }
        let instance = self.instance_ref(CallKind::Callback);
        let ctx = self.require_ctx()?;
        let handle = ctx.scheduler().interval(
            Duration::from_millis(period_ms),
            Box::new(move || {
                proxies::dispatch_scheduled_task(instance.clone(), callback_id);
            }),
        );
        self.record_task(handle.as_u64());
        Ok(handle.as_u64())
    }

    async fn cancel(&mut self, handle: u64) -> wasmtime::Result<()> {
        if self.lacks(Capability::Scheduler, "scheduler.cancel") {
            return Ok(());
        }
        self.forget_task(handle);
        if let Some(ctx) = self.ctx() {
            ctx.scheduler().cancel(TaskHandle::new(handle));
        }
        Ok(())
    }
}
impl limbo::Host for PluginStoreState {
    async fn register_limbo_handler(&mut self, name: String, handler: u64) -> wasmtime::Result<()> {
        if self.lacks(Capability::Limbo, "limbo.register-limbo-handler") {
            return Ok(());
        }
        let instance = self.instance_ref(CallKind::Callback).any_generation();
        let ctx = self.require_ctx()?;
        let Bound::Fresh(binding) =
            self.registrations()
                .bind_limbo(&name, self.generation(), handler)
        else {
            return Ok(());
        };
        ctx.register_limbo_handler(Box::new(crate::limbo::WasmLimboHandler::new(
            binding,
            name,
            instance,
            Arc::clone(self.registrations()),
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
    use infrarust_api::permissions::{CapabilitySet, PermissionLevel};
    use infrarust_api::player::Player;
    use infrarust_api::types::{
        Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, TitleData,
    };

    use super::*;
    use crate::bindings::infrarust::plugin::player_registry::{Host as _, HostPlayer as _};
    use crate::config::SandboxLimits;
    use crate::deadline::Deadline;
    use crate::store_state::build_probe_state;

    #[tokio::test]
    async fn get_player_by_uuid_traps_on_malformed_uuid() {
        let mut state = build_probe_state("test".to_string(), &SandboxLimits::default())
            .with_capabilities(CapabilitySet::baseline());
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
        let mut state = build_probe_state("test".to_string(), &SandboxLimits::default())
            .with_capabilities(CapabilitySet::baseline());
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

    #[tokio::test]
    async fn switch_server_stops_at_the_call_deadline_when_that_comes_first() {
        let mut state = build_probe_state("test".to_string(), &SandboxLimits::default())
            .with_capabilities(CapabilitySet::baseline());
        let handle = state
            .push_player(Arc::new(StalledPlayer::new()))
            .expect("the resource table accepts a player");
        state.begin_call(Some(Deadline::after(Duration::ZERO)));

        let result = state
            .switch_server(handle, "lobby".to_string())
            .await
            .expect("a call out of time is a player-error, not a trap");

        assert!(
            matches!(&result, Err(wt::PlayerError::SwitchFailed(m)) if m.contains("deadline")),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn player_write_calls_are_refused_without_the_capability() {
        let mut state = build_probe_state("test".to_string(), &SandboxLimits::default())
            .with_capabilities(CapabilitySet::baseline().without(Capability::PlayerWrite));
        let player = Arc::new(StalledPlayer::new());

        let handle = state.push_player(player.clone()).unwrap();
        let switched = state
            .switch_server(handle, "lobby".to_string())
            .await
            .unwrap();
        assert!(
            matches!(&switched, Err(wt::PlayerError::SwitchFailed(m)) if m.contains("player-write")),
            "{switched:?}"
        );

        let handle = state.push_player(player.clone()).unwrap();
        let sent = state.send_message(handle, "hi".to_string()).await.unwrap();
        assert!(
            matches!(&sent, Err(wt::PlayerError::SendFailed(m)) if m.contains("player-write")),
            "{sent:?}"
        );

        let handle = state.push_player(player).unwrap();
        state
            .disconnect(handle, "bye".to_string())
            .await
            .expect("a denied disconnect is ignored, not a trap");
    }

    macro_rules! expect {
        ($failures:ident, $capability:expr, $call:literal, $result:expr, $pattern:pat $(if $guard:expr)?) => {{
            let result = $result;
            if !matches!(result, $pattern $(if $guard)?) {
                $failures.push(format!("{:?} {}: {:?}", $capability, $call, result));
            }
        }};
    }

    fn nobody() -> wt::BanTarget {
        wt::BanTarget::Username("nobody".to_string())
    }

    fn missing(capability: Capability) -> String {
        format!("missing capability: {}", capability.to_kebab())
    }

    fn codec_metadata() -> codec_registry::CodecFilterMetadata {
        codec_registry::CodecFilterMetadata {
            id: "ops".to_string(),
            priority: 2,
            after: vec![],
            before: vec![],
        }
    }

    async fn unrefused_calls(capability: Capability) -> Vec<String> {
        let mut state = build_probe_state("denied".to_string(), &SandboxLimits::default())
            .with_capabilities(CapabilitySet::native_trusted().without(capability));
        let denied = missing(capability);
        let mut failures = Vec::new();
        let s = &mut state;
        match capability {
            Capability::Ban => {
                expect!(failures, capability, "ban", ban_service::Host::ban(s, nobody(), None, None).await,
                    Ok(Err(wt::ServiceError::OperationFailed(ref m))) if *m == denied);
                expect!(failures, capability, "unban", ban_service::Host::unban(s, nobody()).await,
                    Ok(Err(wt::ServiceError::OperationFailed(ref m))) if *m == denied);
                expect!(failures, capability, "is-banned", ban_service::Host::is_banned(s, nobody()).await,
                    Ok(Err(wt::ServiceError::OperationFailed(ref m))) if *m == denied);
                expect!(failures, capability, "get-ban", ban_service::Host::get_ban(s, nobody()).await,
                    Ok(Err(wt::ServiceError::OperationFailed(ref m))) if *m == denied);
                expect!(failures, capability, "get-all-bans", ban_service::Host::get_all_bans(s).await,
                    Ok(Err(wt::ServiceError::OperationFailed(ref m))) if *m == denied);
            }
            Capability::ServerManage => {
                expect!(failures, capability, "start", server_manager::Host::start(s, "lobby".into()).await,
                    Ok(Err(wt::ServiceError::OperationFailed(ref m))) if *m == denied);
                expect!(failures, capability, "stop", server_manager::Host::stop(s, "lobby".into()).await,
                    Ok(Err(wt::ServiceError::OperationFailed(ref m))) if *m == denied);
                expect!(
                    failures,
                    capability,
                    "get-state",
                    server_manager::Host::get_state(s, "lobby".into()).await,
                    Ok(None)
                );
                expect!(failures, capability, "get-all-servers", server_manager::Host::get_all_servers(s).await,
                    Ok(ref v) if v.is_empty());
            }
            Capability::ConfigRead => {
                expect!(
                    failures,
                    capability,
                    "get-server-config",
                    config_service::Host::get_server_config(s, "lobby".into()).await,
                    Ok(None)
                );
                expect!(failures, capability, "get-all-server-configs",
                    config_service::Host::get_all_server_configs(s).await, Ok(ref v) if v.is_empty());
                expect!(
                    failures,
                    capability,
                    "get-value",
                    config_service::Host::get_value(s, "greeting".into()).await,
                    Ok(None)
                );
            }
            Capability::PlayerRead => {
                expect!(
                    failures,
                    capability,
                    "get-player",
                    player_registry::Host::get_player(s, "Steve".into()).await,
                    Ok(None)
                );
                expect!(
                    failures,
                    capability,
                    "get-player-by-uuid",
                    player_registry::Host::get_player_by_uuid(s, uuid::Uuid::nil().to_string())
                        .await,
                    Ok(None)
                );
                expect!(
                    failures,
                    capability,
                    "get-player-by-id",
                    player_registry::Host::get_player_by_id(s, 1).await,
                    Ok(None)
                );
                expect!(failures, capability, "get-players-on-server",
                    player_registry::Host::get_players_on_server(s, "lobby".into()).await, Ok(ref v) if v.is_empty());
                expect!(failures, capability, "get-all-players",
                    player_registry::Host::get_all_players(s).await, Ok(ref v) if v.is_empty());
                expect!(
                    failures,
                    capability,
                    "online-count",
                    player_registry::Host::online_count(s).await,
                    Ok(0)
                );
                expect!(
                    failures,
                    capability,
                    "online-count-on",
                    player_registry::Host::online_count_on(s, "lobby".into()).await,
                    Ok(0)
                );
            }
            Capability::PlayerWrite => {
                let player: Arc<dyn Player> = Arc::new(StalledPlayer::new());
                let handle = s.push_player(Arc::clone(&player)).unwrap();
                expect!(failures, capability, "send-message",
                    player_registry::HostPlayer::send_message(s, handle, "hi".into()).await,
                    Ok(Err(wt::PlayerError::SendFailed(ref m))) if *m == denied);
                let handle = s.push_player(Arc::clone(&player)).unwrap();
                expect!(failures, capability, "send-action-bar",
                    player_registry::HostPlayer::send_action_bar(s, handle, "hi".into()).await,
                    Ok(Err(wt::PlayerError::SendFailed(ref m))) if *m == denied);
                let handle = s.push_player(Arc::clone(&player)).unwrap();
                let title = wt::TitleData {
                    title: "t".into(),
                    subtitle: "s".into(),
                    fade_in_ticks: 0,
                    stay_ticks: 0,
                    fade_out_ticks: 0,
                };
                expect!(failures, capability, "send-title",
                    player_registry::HostPlayer::send_title(s, handle, title).await,
                    Ok(Err(wt::PlayerError::SendFailed(ref m))) if *m == denied);
                let handle = s.push_player(Arc::clone(&player)).unwrap();
                expect!(failures, capability, "switch-server",
                    player_registry::HostPlayer::switch_server(s, handle, "lobby".into()).await,
                    Ok(Err(wt::PlayerError::SwitchFailed(ref m))) if *m == denied);
                let handle = s.push_player(player).unwrap();
                expect!(
                    failures,
                    capability,
                    "disconnect",
                    player_registry::HostPlayer::disconnect(s, handle, "bye".into()).await,
                    Ok(())
                );
            }
            Capability::RawPacket => {
                let handle = s.push_player(Arc::new(StalledPlayer::new())).unwrap();
                let packet = wt::RawPacket {
                    packet_id: 1,
                    data: vec![],
                };
                expect!(failures, capability, "send-packet",
                    player_registry::HostPlayer::send_packet(s, handle, packet).await,
                    Ok(Err(wt::PlayerError::SendFailed(ref m))) if *m == denied);
                if let Err(e) =
                    event_bus::Host::subscribe(s, event_bus::EventKind::RawPacket, 128).await
                {
                    failures.push(format!("{capability:?} subscribe(raw-packet): {e:?}"));
                }
            }
            Capability::EventBus => {
                match event_bus::Host::subscribe(s, event_bus::EventKind::PostLogin, 128).await {
                    Ok(listener) => {
                        if s.take_listener(listener).is_some() {
                            failures
                                .push(format!("{capability:?} subscribe registered a listener"));
                        }
                        expect!(
                            failures,
                            capability,
                            "unsubscribe",
                            event_bus::Host::unsubscribe(s, listener).await,
                            Ok(())
                        );
                    }
                    Err(e) => failures.push(format!("{capability:?} subscribe: {e:?}")),
                }
            }
            Capability::Command => {
                expect!(
                    failures,
                    capability,
                    "register",
                    command_manager::Host::register(s, "probe".into(), vec![], String::new(), 1)
                        .await,
                    Ok(())
                );
                expect!(
                    failures,
                    capability,
                    "unregister",
                    command_manager::Host::unregister(s, "probe".into()).await,
                    Ok(())
                );
            }
            Capability::Scheduler => {
                expect!(
                    failures,
                    capability,
                    "delay",
                    scheduler::Host::delay(s, 10, 1).await,
                    Ok(0)
                );
                expect!(
                    failures,
                    capability,
                    "interval",
                    scheduler::Host::interval(s, 10, 1).await,
                    Ok(0)
                );
                expect!(
                    failures,
                    capability,
                    "cancel",
                    scheduler::Host::cancel(s, 0).await,
                    Ok(())
                );
            }
            Capability::CodecFilter => {
                expect!(
                    failures,
                    capability,
                    "register-codec-filter",
                    codec_registry::Host::register_codec_filter(s, codec_metadata(), 1).await,
                    Ok(())
                );
                expect!(
                    failures,
                    capability,
                    "unregister-codec-filter",
                    codec_registry::Host::unregister_codec_filter(s, "ops".into()).await,
                    Ok(())
                );
            }
            Capability::Limbo => {
                expect!(
                    failures,
                    capability,
                    "register-limbo-handler",
                    limbo::Host::register_limbo_handler(s, "gate".into(), 1).await,
                    Ok(())
                );
            }
            other => failures.push(format!("{other:?}: no gated host call to probe")),
        }
        failures
    }

    #[tokio::test]
    async fn every_gated_host_call_is_refused_without_its_capability() {
        let mut failures = Vec::new();
        for capability in [
            Capability::Ban,
            Capability::ServerManage,
            Capability::ConfigRead,
            Capability::PlayerRead,
            Capability::PlayerWrite,
            Capability::RawPacket,
            Capability::EventBus,
            Capability::Command,
            Capability::Scheduler,
            Capability::CodecFilter,
            Capability::Limbo,
        ] {
            failures.extend(unrefused_calls(capability).await);
        }
        assert!(
            failures.is_empty(),
            "host calls that were not refused:\n{}",
            failures.join("\n")
        );
    }
}
