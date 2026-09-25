use std::sync::Arc;

use crate::event::Event;
use crate::events::packet::PacketDirection;
use crate::player::{ClientSettings, Player};
use crate::types::PlayerId;

#[non_exhaustive]
pub struct PlayerClientBrandEvent {
    pub player: Arc<dyn Player>,
    pub brand: String,
}

impl PlayerClientBrandEvent {
    pub fn new(player: Arc<dyn Player>, brand: String) -> Self {
        Self { player, brand }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for PlayerClientBrandEvent {}

#[non_exhaustive]
pub struct PlayerSettingsChangedEvent {
    pub player: Arc<dyn Player>,
    pub settings: ClientSettings,
}

impl PlayerSettingsChangedEvent {
    pub fn new(player: Arc<dyn Player>, settings: ClientSettings) -> Self {
        Self { player, settings }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for PlayerSettingsChangedEvent {}

#[non_exhaustive]
pub struct PlayerChannelRegisterEvent {
    pub player: Arc<dyn Player>,
    pub channels: Vec<String>,
    pub direction: PacketDirection,
}

impl PlayerChannelRegisterEvent {
    pub fn new(player: Arc<dyn Player>, channels: Vec<String>, direction: PacketDirection) -> Self {
        Self {
            player,
            channels,
            direction,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for PlayerChannelRegisterEvent {}
