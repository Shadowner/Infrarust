use std::sync::Arc;

use infrarust_api::permissions::{Capability, PermissionSnapshot};
use infrarust_api::player::Player;
use infrarust_api::types::PlayerId;

use crate::bindings::infrarust::plugin::permissions as wp;
use crate::bindings::infrarust::plugin::types::ErrorKind;
use crate::deadline::HostCallLimit;
use crate::host_error::{HostResult, host_error, timed_out};
use crate::snapshots::snapshot_from_wit;
use crate::store_state::PluginStoreState;

impl wp::Host for PluginStoreState {
    async fn set_snapshot(
        &mut self,
        player: u64,
        snapshot: wp::PermissionSnapshot,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.set_player_snapshot(player, &snapshot).await)
    }

    async fn release(&mut self, player: u64) -> wasmtime::Result<HostResult<()>> {
        Ok(self.release_player_snapshot(player).await)
    }
}

impl PluginStoreState {
    async fn set_player_snapshot(
        &mut self,
        player: u64,
        snapshot: &wp::PermissionSnapshot,
    ) -> HostResult<()> {
        self.check(Capability::PermissionProvider, "permissions.set-snapshot")?;
        let snapshot = snapshot_from_wit(snapshot)
            .map_err(|reason| host_error(ErrorKind::InvalidArgument, reason))?;
        let online = self.online_player(player)?;
        if !self
            .registrations()
            .snapshots()
            .update(PlayerId::new(player), snapshot)
        {
            return Err(host_error(
                ErrorKind::NotFound,
                format!(
                    "player {player} holds no permission snapshot from this plugin; answer permissions-setup with custom or provide permissions first"
                ),
            ));
        }
        refresh(self.service_call_limit(), online).await
    }

    async fn release_player_snapshot(&mut self, player: u64) -> HostResult<()> {
        self.check(Capability::PermissionProvider, "permissions.release")?;
        let Some(released) = self
            .registrations()
            .snapshots()
            .release(PlayerId::new(player))
        else {
            return Ok(());
        };
        released.replace(PermissionSnapshot::new());
        match self.online_player(player) {
            Ok(online) => refresh(self.service_call_limit(), online).await,
            Err(_) => Ok(()),
        }
    }
}

async fn refresh(limit: HostCallLimit, player: Arc<dyn Player>) -> HostResult<()> {
    limit
        .run(player.refresh_permissions())
        .await
        .map_err(timed_out)
}
