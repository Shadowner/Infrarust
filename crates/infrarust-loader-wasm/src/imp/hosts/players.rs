use std::sync::Arc;

use infrarust_api::permissions::Capability;
use infrarust_api::player::Player;
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::{PlayerId, ServerId};
use tokio::time::timeout;

use super::parse_text;
use crate::bindings::infrarust::plugin::players as wp;
use crate::bindings::infrarust::plugin::types as wt;
use crate::consts::PLAYER_SWITCH_TIMEOUT;
use crate::convert;
use crate::host_error::{HostResult, invalid_component, player_error, timed_out};
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
}
