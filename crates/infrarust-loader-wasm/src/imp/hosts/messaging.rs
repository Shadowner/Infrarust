use bytes::Bytes;
use infrarust_api::types::ServerId;

use crate::bindings::infrarust::plugin::messaging as wm;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::host_error::{HostResult, messaging_error, player_error};
use crate::store_state::{PluginStoreState, Quota};

impl wm::Host for PluginStoreState {
    async fn register_channel(
        &mut self,
        channel: wt::ChannelId,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.register_plugin_channel(&channel))
    }

    async fn unregister_channel(
        &mut self,
        channel: wt::ChannelId,
    ) -> wasmtime::Result<HostResult<bool>> {
        Ok(self.unregister_plugin_channel(&channel))
    }

    async fn channels(&mut self) -> wasmtime::Result<HostResult<Vec<wt::ChannelId>>> {
        Ok(self.plugin_channels())
    }

    async fn send_to_player(
        &mut self,
        player: u64,
        channel: wt::ChannelId,
        data: Vec<u8>,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.message_player_channel(player, &channel, data))
    }

    async fn send_to_backend(
        &mut self,
        player: u64,
        channel: wt::ChannelId,
        data: Vec<u8>,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.message_backend_channel(player, &channel, data))
    }

    async fn send_to_server(
        &mut self,
        server: String,
        channel: wt::ChannelId,
        data: Vec<u8>,
    ) -> wasmtime::Result<HostResult<u32>> {
        Ok(self.message_server_channel(server, &channel, data))
    }
}

impl PluginStoreState {
    fn register_plugin_channel(&mut self, channel: &wt::ChannelId) -> HostResult<()> {
        self.check("messaging", "register-channel")?;
        let channel = convert::channel_from_wit(channel)?;
        let ctx = self.services()?;
        let held = ctx.channel_registrar().channels();
        if !held.contains(&channel) {
            self.admit(Quota::PluginChannels, held.len(), 1)?;
        }
        ctx.channel_registrar().register(channel);
        Ok(())
    }

    fn unregister_plugin_channel(&mut self, channel: &wt::ChannelId) -> HostResult<bool> {
        self.check("messaging", "unregister-channel")?;
        let channel = convert::channel_from_wit(channel)?;
        Ok(self.services()?.channel_registrar().unregister(&channel))
    }

    fn plugin_channels(&mut self) -> HostResult<Vec<wt::ChannelId>> {
        self.check("messaging", "channels")?;
        Ok(self
            .services()?
            .channel_registrar()
            .channels()
            .iter()
            .map(convert::channel_to_wit)
            .collect())
    }

    fn message_player_channel(
        &mut self,
        player: u64,
        channel: &wt::ChannelId,
        data: Vec<u8>,
    ) -> HostResult<()> {
        self.check("messaging", "send-to-player")?;
        let channel = convert::channel_from_wit(channel)?;
        self.online_player(player)?
            .send_plugin_message(&channel, Bytes::from(data))
            .map_err(player_error)
    }

    fn message_backend_channel(
        &mut self,
        player: u64,
        channel: &wt::ChannelId,
        data: Vec<u8>,
    ) -> HostResult<()> {
        self.check("messaging", "send-to-backend")?;
        let channel = convert::channel_from_wit(channel)?;
        self.online_player(player)?
            .send_plugin_message_to_backend(&channel, Bytes::from(data))
            .map_err(player_error)
    }

    fn message_server_channel(
        &mut self,
        server: String,
        channel: &wt::ChannelId,
        data: Vec<u8>,
    ) -> HostResult<u32> {
        self.check("messaging", "send-to-server")?;
        let channel = convert::channel_from_wit(channel)?;
        let carriers = self
            .services()?
            .server_messenger()
            .send_to_server(&ServerId::from(server), &channel, Bytes::from(data))
            .map_err(|e| messaging_error(&e))?;
        Ok(u32::try_from(carriers).unwrap_or(u32::MAX))
    }
}
