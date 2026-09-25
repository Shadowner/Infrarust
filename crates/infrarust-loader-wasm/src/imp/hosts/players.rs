use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::error::PlayerError;
use infrarust_api::permissions::Capability;
use infrarust_api::player::{
    BossBar, BossBarColor, BossBarFlags, BossBarOverlay, BossBarUpdate, ConnectionResult, Player,
    ResourcePackRequest,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::{PlayerId, ServerId};
use tokio::time::timeout;

use super::parse_text;
use crate::bindings::infrarust::plugin::players as wp;
use crate::bindings::infrarust::plugin::types as wt;
use crate::component;
use crate::consts::{MAX_BOSS_BARS, PLAYER_SWITCH_TIMEOUT};
use crate::convert;
use crate::host_error::{HostResult, host_error, invalid_component, player_error, timed_out};
use crate::store_state::PluginStoreState;

pub(crate) fn player_info(player: &dyn Player) -> wp::PlayerInfo {
    wp::PlayerInfo {
        player: convert::player_ref(player),
        profile: convert::game_profile_to_wit(player.profile()),
        protocol: player.protocol_version().raw(),
        remote_addr: convert::socket_to_wit(player.remote_addr()),
        current_server: player.current_server().map(|s| s.as_str().to_owned()),
        online_mode: player.is_online_mode(),
        connected: player.is_connected(),
        active: player.is_active(),
        connected_at: convert::system_time_to_millis(player.connected_at()),
        virtual_host: player.virtual_host(),
        client_brand: player.client_brand(),
        ping_ms: player
            .ping()
            .map(|ping| u32::try_from(ping.as_millis()).unwrap_or(u32::MAX)),
        settings: player
            .settings()
            .as_ref()
            .map(convert::client_settings_to_wit),
        known_channels: player.known_channels(),
    }
}

fn connection_result_to_wit(result: &ConnectionResult) -> wp::ConnectionResult {
    match result {
        ConnectionResult::Success => wp::ConnectionResult::Success,
        ConnectionResult::AlreadyConnected => wp::ConnectionResult::AlreadyConnected,
        ConnectionResult::Denied(reason) => wp::ConnectionResult::Denied(component::to_wit(reason)),
        ConnectionResult::Failed(reason) => wp::ConnectionResult::Failed(component::to_wit(reason)),
        _ => wp::ConnectionResult::Cancelled,
    }
}

const fn bar_color(color: wp::BossBarColor) -> BossBarColor {
    match color {
        wp::BossBarColor::Pink => BossBarColor::Pink,
        wp::BossBarColor::Blue => BossBarColor::Blue,
        wp::BossBarColor::Red => BossBarColor::Red,
        wp::BossBarColor::Green => BossBarColor::Green,
        wp::BossBarColor::Yellow => BossBarColor::Yellow,
        wp::BossBarColor::Purple => BossBarColor::Purple,
        wp::BossBarColor::White => BossBarColor::White,
    }
}

const fn bar_overlay(overlay: wp::BossBarOverlay) -> BossBarOverlay {
    match overlay {
        wp::BossBarOverlay::Progress => BossBarOverlay::Progress,
        wp::BossBarOverlay::Notched6 => BossBarOverlay::Notched6,
        wp::BossBarOverlay::Notched10 => BossBarOverlay::Notched10,
        wp::BossBarOverlay::Notched12 => BossBarOverlay::Notched12,
        wp::BossBarOverlay::Notched20 => BossBarOverlay::Notched20,
    }
}

fn bar_flags(flags: wp::BossBarFlags) -> BossBarFlags {
    [
        (wp::BossBarFlags::DARKEN_SCREEN, BossBarFlags::DARKEN_SCREEN),
        (
            wp::BossBarFlags::PLAY_BOSS_MUSIC,
            BossBarFlags::PLAY_BOSS_MUSIC,
        ),
        (
            wp::BossBarFlags::CREATE_WORLD_FOG,
            BossBarFlags::CREATE_WORLD_FOG,
        ),
    ]
    .into_iter()
    .filter(|(flag, _)| flags.contains(*flag))
    .fold(BossBarFlags::NONE, |set, (_, bit)| set.with(bit))
}

fn bar_update(update: &wp::BossBarUpdate) -> HostResult<BossBarUpdate> {
    Ok(match update {
        wp::BossBarUpdate::Title(title) => BossBarUpdate::Title(parse_text(title)?),
        wp::BossBarUpdate::Progress(progress) => BossBarUpdate::Progress(*progress),
        wp::BossBarUpdate::Style((color, overlay)) => BossBarUpdate::Style {
            color: bar_color(*color),
            overlay: bar_overlay(*overlay),
        },
        wp::BossBarUpdate::Flags(flags) => BossBarUpdate::Flags(bar_flags(*flags)),
    })
}

fn resource_pack(pack: &wp::ResourcePackRequest) -> HostResult<ResourcePackRequest> {
    let mut request = ResourcePackRequest::new(pack.url.clone())
        .id(convert::uuid_from_wit(pack.id))
        .required(pack.required);
    if let Some(hash) = &pack.hash {
        request = request.hash(hash.clone());
    }
    if let Some(prompt) = &pack.prompt {
        request = request.prompt(parse_text(prompt)?);
    }
    request
        .validate()
        .map_err(|reason| host_error(wt::ErrorKind::InvalidArgument, reason))?;
    Ok(request)
}

async fn bounded<T>(
    limit: crate::deadline::HostCallLimit,
    call: impl Future<Output = Result<T, PlayerError>>,
) -> HostResult<T> {
    match limit.run(call).await {
        Ok(result) => result.map_err(player_error),
        Err(expired) => Err(timed_out(expired)),
    }
}

fn infos(players: &[Arc<dyn Player>]) -> Vec<wp::PlayerInfo> {
    players
        .iter()
        .map(|player| player_info(&**player))
        .collect()
}

impl PluginStoreState {
    fn readable(&mut self, call: &'static str) -> Option<Arc<dyn PluginContext>> {
        if self.lacks(Capability::PlayerRead, call) {
            return None;
        }
        self.services().ok()
    }

    fn writable_player(&mut self, call: &'static str, id: u64) -> HostResult<Arc<dyn Player>> {
        self.check(Capability::PlayerWrite, call)?;
        self.online_player(id)
    }

    fn show_bar(&mut self, player: u64, bar: &wp::BossBar) -> HostResult<wt::Uuid> {
        let player = self.writable_player("players.show-boss-bar", player)?;
        if self.boss_bar_count() >= MAX_BOSS_BARS {
            return Err(host_error(
                wt::ErrorKind::Conflict,
                format!(
                    "a plugin can show at most {MAX_BOSS_BARS} boss bars at once; hide one first"
                ),
            ));
        }
        let native = BossBar::new(parse_text(&bar.title)?)
            .progress(bar.progress)
            .color(bar_color(bar.color))
            .overlay(bar_overlay(bar.overlay))
            .flags(bar_flags(bar.flags));
        let handle = player.show_boss_bar(native).map_err(player_error)?;
        let id = convert::uuid_to_wit(handle.id());
        self.record_boss_bar(handle);
        Ok(id)
    }

    fn update_bar(&mut self, bar: wt::Uuid, update: &wp::BossBarUpdate) -> HostResult<()> {
        self.check(Capability::PlayerWrite, "players.update-boss-bar")?;
        let id = convert::uuid_from_wit(bar);
        let update = bar_update(update)?;
        let result = self
            .boss_bar(id)
            .ok_or_else(|| unknown_bar(id))?
            .update(update);
        if matches!(result, Err(PlayerError::Disconnected)) {
            self.forget_boss_bar(id);
        }
        result.map_err(player_error)
    }

    fn hide_bar(&mut self, bar: wt::Uuid) -> HostResult<()> {
        self.check(Capability::PlayerWrite, "players.hide-boss-bar")?;
        let id = convert::uuid_from_wit(bar);
        let handle = self.forget_boss_bar(id).ok_or_else(|| unknown_bar(id))?;
        match handle.hide() {
            Ok(()) | Err(PlayerError::Disconnected) => Ok(()),
            Err(error) => Err(player_error(error)),
        }
    }
}

fn unknown_bar(id: uuid::Uuid) -> wt::HostError {
    host_error(
        wt::ErrorKind::NotFound,
        format!("boss bar {id} is not shown by this plugin"),
    )
}

impl wp::Host for PluginStoreState {
    async fn get(&mut self, id: u64) -> wasmtime::Result<Option<wp::PlayerInfo>> {
        Ok(self.readable("players.get").and_then(|ctx| {
            ctx.player_registry()
                .get_player_by_id(PlayerId::new(id))
                .map(|player| player_info(&*player))
        }))
    }

    async fn get_by_name(&mut self, username: String) -> wasmtime::Result<Option<wp::PlayerInfo>> {
        Ok(self.readable("players.get-by-name").and_then(|ctx| {
            ctx.player_registry()
                .get_player(&username)
                .map(|player| player_info(&*player))
        }))
    }

    async fn get_by_uuid(&mut self, id: wt::Uuid) -> wasmtime::Result<Option<wp::PlayerInfo>> {
        Ok(self.readable("players.get-by-uuid").and_then(|ctx| {
            ctx.player_registry()
                .get_player_by_uuid(&convert::uuid_from_wit(id))
                .map(|player| player_info(&*player))
        }))
    }

    async fn list(&mut self, server: Option<String>) -> wasmtime::Result<Vec<wp::PlayerInfo>> {
        Ok(self
            .readable("players.list")
            .map(|ctx| {
                let registry = ctx.player_registry();
                match server {
                    Some(server) => infos(&registry.get_players_on_server(&ServerId::from(server))),
                    None => infos(&registry.get_all_players()),
                }
            })
            .unwrap_or_default())
    }

    async fn count(&mut self, server: Option<String>) -> wasmtime::Result<u32> {
        Ok(self
            .readable("players.count")
            .map(|ctx| {
                let registry = ctx.player_registry();
                let count = match server {
                    Some(server) => registry.online_count_on(&ServerId::from(server)),
                    None => registry.online_count(),
                };
                u32::try_from(count).unwrap_or(u32::MAX)
            })
            .unwrap_or(0))
    }

    async fn send_message(
        &mut self,
        player: u64,
        message: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.send-message", player)?;
            let message = parse_text(&message)?;
            player.send_message(message).map_err(player_error)
        })())
    }

    async fn send_title(
        &mut self,
        player: u64,
        title: wt::TitleData,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.send-title", player)?;
            let title = convert::title_data_from_wit(&title).map_err(|e| invalid_component(&e))?;
            player.send_title(title).map_err(player_error)
        })())
    }

    async fn send_action_bar(
        &mut self,
        player: u64,
        message: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.send-action-bar", player)?;
            let message = parse_text(&message)?;
            player.send_action_bar(message).map_err(player_error)
        })())
    }

    async fn send_packet(
        &mut self,
        player: u64,
        packet: wt::RawPacket,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            self.check(Capability::RawPacket, "players.send-packet")?;
            let player = self.online_player(player)?;
            player
                .send_packet(convert::raw_packet_from_wit(packet))
                .map_err(player_error)
        })())
    }

    async fn disconnect(
        &mut self,
        player: u64,
        reason: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.disconnect", player)?;
            let reason = parse_text(&reason)?;
            let plugin_id = self.plugin_id.clone();
            let limit = self.host_call_timeout();
            tokio::spawn(async move {
                let player_id = player.id().as_u64();
                if timeout(limit, player.disconnect(reason)).await.is_err() {
                    tracing::warn!(plugin = %plugin_id, player = player_id,
                        "player disconnect requested by plugin timed out");
                }
            });
            Ok(())
        })())
    }

    async fn switch_server(
        &mut self,
        player: u64,
        server: String,
    ) -> wasmtime::Result<HostResult<()>> {
        let player = match self.writable_player("players.switch-server", player) {
            Ok(player) => player,
            Err(error) => return Ok(Err(error)),
        };
        let limit = self.host_call_limit(PLAYER_SWITCH_TIMEOUT);
        Ok(
            match limit
                .run(player.switch_server(ServerId::from(server)))
                .await
            {
                Ok(result) => result.map_err(player_error),
                Err(expired) => Err(timed_out(expired)),
            },
        )
    }

    async fn has_permission(
        &mut self,
        player: u64,
        permission: String,
    ) -> wasmtime::Result<HostResult<bool>> {
        Ok((|| {
            self.check(Capability::PlayerRead, "players.has-permission")?;
            Ok(self.online_player(player)?.has_permission(&permission))
        })())
    }

    async fn connect(
        &mut self,
        player: u64,
        server: String,
    ) -> wasmtime::Result<HostResult<wp::ConnectionResult>> {
        let player = match self.writable_player("players.connect", player) {
            Ok(player) => player,
            Err(error) => return Ok(Err(error)),
        };
        let limit = self.service_call_limit();
        Ok(bounded(limit, player.connect(ServerId::from(server)))
            .await
            .map(|result| connection_result_to_wit(&result)))
    }

    async fn set_player_list_header_footer(
        &mut self,
        player: u64,
        header: wt::Component,
        footer: wt::Component,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.set-player-list-header-footer", player)?;
            let header = parse_text(&header)?;
            let footer = parse_text(&footer)?;
            player
                .set_player_list_header_footer(header, footer)
                .map_err(player_error)
        })())
    }

    async fn clear_title(&mut self, player: u64, reset: bool) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.clear-title", player)?;
            player.clear_title(reset).map_err(player_error)
        })())
    }

    async fn show_boss_bar(
        &mut self,
        player: u64,
        bar: wp::BossBar,
    ) -> wasmtime::Result<HostResult<wt::Uuid>> {
        Ok(self.show_bar(player, &bar))
    }

    async fn update_boss_bar(
        &mut self,
        bar: wt::Uuid,
        update: wp::BossBarUpdate,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.update_bar(bar, &update))
    }

    async fn hide_boss_bar(&mut self, bar: wt::Uuid) -> wasmtime::Result<HostResult<()>> {
        Ok(self.hide_bar(bar))
    }

    async fn send_resource_pack(
        &mut self,
        player: u64,
        pack: wp::ResourcePackRequest,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.send-resource-pack", player)?;
            player
                .send_resource_pack(resource_pack(&pack)?)
                .map_err(player_error)
        })())
    }

    async fn remove_resource_pack(
        &mut self,
        player: u64,
        id: Option<wt::Uuid>,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.remove-resource-pack", player)?;
            player
                .remove_resource_pack(id.map(convert::uuid_from_wit))
                .map_err(player_error)
        })())
    }

    async fn transfer(
        &mut self,
        player: u64,
        target: wt::ServerAddress,
    ) -> wasmtime::Result<HostResult<()>> {
        let player = match self.writable_player("players.transfer", player) {
            Ok(player) => player,
            Err(error) => return Ok(Err(error)),
        };
        let limit = self.service_call_limit();
        Ok(bounded(limit, player.transfer(&target.host, target.port)).await)
    }

    async fn store_cookie(
        &mut self,
        player: u64,
        key: String,
        data: Vec<u8>,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            let player = self.writable_player("players.store-cookie", player)?;
            player
                .store_cookie(&key, Bytes::from(data))
                .map_err(player_error)
        })())
    }

    async fn request_cookie(
        &mut self,
        player: u64,
        key: String,
    ) -> wasmtime::Result<HostResult<Option<Vec<u8>>>> {
        let player = match self.writable_player("players.request-cookie", player) {
            Ok(player) => player,
            Err(error) => return Ok(Err(error)),
        };
        let limit = self.service_call_limit();
        Ok(bounded(limit, player.request_cookie(&key))
            .await
            .map(|cookie| cookie.map(|data| data.to_vec())))
    }

    async fn refresh_permissions(&mut self, player: u64) -> wasmtime::Result<HostResult<()>> {
        let player = match self.writable_player("players.refresh-permissions", player) {
            Ok(player) => player,
            Err(error) => return Ok(Err(error)),
        };
        let limit = self.service_call_limit();
        Ok(bounded(limit, async {
            player.refresh_permissions().await;
            Ok(())
        })
        .await)
    }
}
