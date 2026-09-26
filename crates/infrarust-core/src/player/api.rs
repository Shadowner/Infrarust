use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use tokio::sync::oneshot;
use uuid::Uuid;

use infrarust_api::error::PlayerError;
use infrarust_api::event::BoxFuture;
use infrarust_api::events::connection::ConnectCause;
use infrarust_api::messaging::{ChannelId, MAX_TO_BACKEND_PAYLOAD, MAX_TO_CLIENT_PAYLOAD};
use infrarust_api::player::{
    BossBar, BossBarControl, BossBarHandle, ClientSettings, ConnectionResult, MAX_COOKIE_SIZE,
    Player, ResourcePackRequest, cookie_key,
};
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};

use super::{
    BOSS_BAR_SINCE, BossBarCommand, COOKIES_SINCE, ClientCommand, MAX_TRANSFER_HOST, MessageTarget,
    PACK_STACK_SINCE, PlayerCommand, PlayerSession, RESOURCE_PACK_SINCE, TAB_LIST_SINCE,
    TRANSFER_SINCE,
};
use crate::session::presentation::BarControl;
use crate::util::sync::{lock, read};

impl infrarust_api::player::private::Sealed for PlayerSession {}

impl Player for PlayerSession {
    fn id(&self) -> PlayerId {
        self.player_id
    }

    fn profile(&self) -> &GameProfile {
        &self.profile
    }

    fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    fn current_server(&self) -> Option<ServerId> {
        read(&self.routing).current.clone()
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn disconnect(&self, reason: Component) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Err(e) = self.command_tx.try_send(PlayerCommand::Kick(reason)) {
                tracing::debug!(
                    player = %self.profile.username,
                    "disconnecting without queueing the kick reason: {e}"
                );
            }
            self.shutdown_token.cancel();
        })
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendMessage(message))
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendTitle(Box::new(title)))
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendActionBar(message))
    }

    fn send_packet(&self, packet: RawPacket) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendPacket(packet))
    }

    fn switch_server(&self, target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
        Box::pin(async move {
            self.send_command(PlayerCommand::SwitchServer(target, ConnectCause::Switch))
                .await
        })
    }

    fn is_online_mode(&self) -> bool {
        self.online_mode
    }

    fn has_permission(&self, permission: &str) -> bool {
        let checker = self.permission_checker();
        match &self.permissions {
            Some(permissions) => permissions.value(checker.as_ref(), permission).is_true(),
            None => checker.has_permission(permission),
        }
    }

    fn refresh_permissions(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some(permissions) = &self.permissions
                && !self.permission_override.load(Ordering::Acquire)
            {
                let checker = permissions.create_checker(&self.permission_subject()).await;
                self.set_permission_checker(checker);
            }
            self.permissions_changed
                .send_modify(|generation| *generation = generation.wrapping_add(1));
        })
    }

    fn connected_at(&self) -> SystemTime {
        self.connected_at
    }

    fn virtual_host(&self) -> Option<String> {
        self.virtual_host.clone()
    }

    fn client_brand(&self) -> Option<String> {
        self.client.brand()
    }

    fn settings(&self) -> Option<ClientSettings> {
        self.client.settings()
    }

    fn known_channels(&self) -> Vec<String> {
        self.client.channels()
    }

    fn ping(&self) -> Option<Duration> {
        self.client.ping()
    }

    fn send_plugin_message(&self, channel: &ChannelId, data: Bytes) -> Result<(), PlayerError> {
        self.send_message_to(MessageTarget::Client, channel, data, MAX_TO_CLIENT_PAYLOAD)
    }

    fn send_plugin_message_to_backend(
        &self,
        channel: &ChannelId,
        data: Bytes,
    ) -> Result<(), PlayerError> {
        if self.active && self.connected_address().is_none() {
            return Err(PlayerError::NoBackend);
        }
        self.send_message_to(
            MessageTarget::Backend,
            channel,
            data,
            MAX_TO_BACKEND_PAYLOAD,
        )
    }

    fn connect(&self, target: ServerId) -> BoxFuture<'_, Result<ConnectionResult, PlayerError>> {
        Box::pin(async move {
            self.ready()?;
            self.refuse_self_wait("connect")?;
            let (reply, result) = oneshot::channel();
            lock(&self.connects).push((target.clone(), reply));
            self.send_command(PlayerCommand::SwitchServer(target, ConnectCause::Switch))
                .await?;
            Ok(result.await.unwrap_or(ConnectionResult::Cancelled))
        })
    }

    fn set_player_list_header_footer(
        &self,
        header: Component,
        footer: Component,
    ) -> Result<(), PlayerError> {
        self.supports(TAB_LIST_SINCE, "tab list headers and footers")?;
        self.try_send_command(PlayerCommand::HeaderFooter(Box::new((
            header.clone(),
            footer.clone(),
        ))))?;
        self.presentation.set_header_footer(header, footer);
        Ok(())
    }

    fn clear_title(&self, reset: bool) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::ClearTitle { reset })
    }

    fn show_boss_bar(&self, bar: BossBar) -> Result<BossBarHandle, PlayerError> {
        self.supports(BOSS_BAR_SINCE, "boss bars")?;
        let id = Uuid::new_v4();
        self.presentation.show_bar(id, bar.clone());
        if let Err(e) = self.try_send_command(PlayerCommand::BossBar(
            id,
            BossBarCommand::Show(Box::new(bar)),
        )) {
            self.presentation.hide_bar(id);
            return Err(e);
        }
        let control = BarControl::new(Arc::clone(&self.presentation), self.command_tx.clone());
        Ok(BossBarHandle::new(
            id,
            Arc::new(control) as Arc<dyn BossBarControl>,
        ))
    }

    fn send_resource_pack(&self, pack: ResourcePackRequest) -> Result<(), PlayerError> {
        self.supports(RESOURCE_PACK_SINCE, "resource packs")?;
        pack.validate()
            .map_err(|e| PlayerError::InvalidArgument(e.to_string()))?;
        self.try_send_command(PlayerCommand::Client(ClientCommand::PushPack(Box::new(
            pack,
        ))))
    }

    fn remove_resource_pack(&self, id: Option<Uuid>) -> Result<(), PlayerError> {
        self.supports(PACK_STACK_SINCE, "removing resource packs")?;
        self.try_send_command(PlayerCommand::Client(ClientCommand::PopPack(id)))
    }

    fn transfer(&self, host: &str, port: u16) -> BoxFuture<'_, Result<(), PlayerError>> {
        let host = host.to_string();
        Box::pin(async move {
            self.supports(TRANSFER_SINCE, "transfers")?;
            if host.is_empty() || host.chars().count() > MAX_TRANSFER_HOST {
                return Err(PlayerError::InvalidArgument(format!(
                    "a transfer host must have 1 to {MAX_TRANSFER_HOST} characters"
                )));
            }
            let (host, port) = self.approve_transfer(host, port).await?;
            self.send_command(PlayerCommand::Client(ClientCommand::Transfer {
                host,
                port,
            }))
            .await
        })
    }

    fn store_cookie(&self, key: &str, data: Bytes) -> Result<(), PlayerError> {
        self.supports(COOKIES_SINCE, "cookies")?;
        let key = cookie_key(key).map_err(|e| PlayerError::InvalidArgument(e.to_string()))?;
        if data.len() > MAX_COOKIE_SIZE {
            return Err(PlayerError::InvalidArgument(format!(
                "a cookie of {} bytes is over the {MAX_COOKIE_SIZE} bytes a client keeps",
                data.len()
            )));
        }
        self.try_send_command(PlayerCommand::Client(ClientCommand::StoreCookie {
            key,
            data,
        }))
    }

    fn request_cookie(&self, key: &str) -> BoxFuture<'_, Result<Option<Bytes>, PlayerError>> {
        let key = cookie_key(key);
        Box::pin(async move {
            self.supports(COOKIES_SINCE, "cookies")?;
            let key = key.map_err(|e| PlayerError::InvalidArgument(e.to_string()))?;
            self.refuse_self_wait("request_cookie")?;
            let (reply, answer) = oneshot::channel();
            self.send_command(PlayerCommand::Client(ClientCommand::RequestCookie {
                key,
                reply,
            }))
            .await?;
            answer.await.map_err(|_| PlayerError::Disconnected)
        })
    }
}
